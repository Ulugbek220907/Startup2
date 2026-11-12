//! Try-on evaluation harness.
//!
//! Runs every photo in a folder through the same steps the server uses
//! (pre-checks, garment mockup render, provider call), measures latency and
//! cost per image, saves the results and writes a report with a manual
//! quality checklist to fill in.
//!
//! ```text
//! cargo run -p evalkit -- --photos eval/photos --design eval/designs/sample.json --out eval/out
//! TRYON_PROVIDER=fashn FASHN_API_KEY=... cargo run -p evalkit -- --photos eval/photos
//! cargo run -p evalkit -- --self-test        # no photos needed, mock provider
//! ```
//!
//! Only use photos of people who agreed to it; the real provider receives
//! them. Results in `eval/out/` are git-ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use design::fonts::EmbeddedFonts;
use design::{Color, DesignDoc, GarmentKind, Side};
use domain::Tiyin;
use providers::tryon::{precheck_photo, FashnConfig, FashnTryOn, MockTryOn, TryOnProvider, TryOnRequest};
use renderer::AreaGeom;

struct Args {
    photos: PathBuf,
    design: Option<PathBuf>,
    out: PathBuf,
    self_test: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args { photos: "eval/photos".into(), design: None, out: "eval/out".into(), self_test: false };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match a.as_str() {
            "--photos" => args.photos = value("--photos")?.into(),
            "--design" => args.design = Some(value("--design")?.into()),
            "--out" => args.out = value("--out")?.into(),
            "--self-test" => args.self_test = true,
            "-h" | "--help" => {
                println!("usage: tryon-eval [--photos DIR] [--design FILE.json] [--out DIR] [--self-test]");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(args)
}

/// Print areas of the seeded garments (front side), so the harness needs no
/// database. Keep in sync with `migrations/*_seed.sql` if those change.
fn front_area(kind: GarmentKind) -> AreaGeom {
    match kind {
        GarmentKind::Tshirt => AreaGeom { width_cm: 30.0, height_cm: 40.0, mock_x: 0.3665, mock_y: 0.2273, mock_w: 0.267, mock_h: 0.3236 },
        GarmentKind::Hoodie => AreaGeom { width_cm: 30.0, height_cm: 35.0, mock_x: 0.381, mock_y: 0.2909, mock_w: 0.238, mock_h: 0.2524 },
        GarmentKind::Sweatshirt => AreaGeom { width_cm: 30.0, height_cm: 40.0, mock_x: 0.377, mock_y: 0.2364, mock_w: 0.246, mock_h: 0.2982 },
    }
}

fn sample_design() -> DesignDoc {
    let mut doc = DesignDoc::new("tshirt", "black");
    doc.front = design::templates::find("city_arch").map(|t| t.layers(30.0, 40.0, Color::WHITE)).unwrap_or_default();
    doc
}

fn provider_from_env() -> Result<Arc<dyn TryOnProvider>> {
    match std::env::var("TRYON_PROVIDER").as_deref().unwrap_or("mock") {
        "mock" => Ok(Arc::new(MockTryOn { delay: Duration::ZERO })),
        "fashn" => {
            let api_key = std::env::var("FASHN_API_KEY").context("FASHN_API_KEY is required for TRYON_PROVIDER=fashn")?;
            let cost_sum: i64 = std::env::var("TRYON_COST_PER_IMAGE_SUM").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
            Ok(Arc::new(FashnTryOn::new(FashnConfig {
                api_key,
                base_url: std::env::var("FASHN_BASE_URL").unwrap_or_else(|_| "https://api.fashn.ai".into()),
                mode: std::env::var("FASHN_MODE").unwrap_or_else(|_| "balanced".into()),
                timeout: Duration::from_secs(180),
                poll_interval: Duration::from_secs(2),
                cost_per_image: Tiyin::from_sum(cost_sum).unwrap_or(Tiyin::ZERO),
            })?))
        }
        other => bail!("unknown TRYON_PROVIDER {other:?} (use mock or fashn)"),
    }
}

fn write_self_test_photo(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    // A bright synthetic "portrait": enough to exercise the pipeline, not a
    // substitute for real photos when judging quality.
    let img = image::RgbImage::from_fn(720, 960, |x, y| image::Rgb([(150 + x / 12) as u8, (170 - y / 16) as u8, 190]));
    img.save(dir.join("synthetic.jpg"))?;
    Ok(())
}

struct Row {
    file: String,
    outcome: String,
    latency_ms: u128,
    cost: Tiyin,
    output: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = parse_args()?;
    if args.self_test {
        args.photos = args.out.join("self-test-photos");
        write_self_test_photo(&args.photos)?;
    }
    let doc = match &args.design {
        Some(path) => design::migrate(serde_json::from_str(&std::fs::read_to_string(path)?)?).map_err(anyhow::Error::msg)?,
        None => sample_design(),
    };
    let kind = GarmentKind::parse(&doc.garment).context("the design's garment must be tshirt, hoodie or sweatshirt for the harness")?;
    let provider = provider_from_env()?;
    std::fs::create_dir_all(&args.out)?;

    // The garment image every photo is tried with (uploaded artwork is not
    // available offline, so image layers are skipped).
    let no_assets: HashMap<String, Arc<image::RgbaImage>> = HashMap::new();
    let garment = renderer::render_mockup(&doc, Side::Front, kind, Color::rgb(24, 24, 27), &front_area(kind), 768, &EmbeddedFonts, &no_assets)?;
    let garment_png = Bytes::from(renderer::encode_png(&garment, None)?);
    std::fs::write(args.out.join("garment.png"), &garment_png)?;

    let mut photos: Vec<PathBuf> = std::fs::read_dir(&args.photos)
        .with_context(|| format!("cannot read {}", args.photos.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("jpg" | "jpeg" | "png" | "webp")))
        .collect();
    photos.sort();
    if photos.is_empty() {
        bail!("no photos in {} (add .jpg/.png files, or run with --self-test)", args.photos.display());
    }

    println!("provider: {} | photos: {} | garment: {}", provider.name(), photos.len(), doc.garment);
    let mut rows = Vec::new();
    for path in &photos {
        let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("photo").to_string();
        let started = Instant::now();
        let outcome = run_one(&*provider, path, &garment_png, &args.out, &file).await;
        let latency_ms = started.elapsed().as_millis();
        let row = match outcome {
            Ok(output) => Row { file, outcome: "ok".into(), latency_ms, cost: provider.cost_per_image(), output: Some(output) },
            // Failed generations are not billed (true for FASHN; see docs).
            Err(e) => Row { file, outcome: format!("{e:#}"), latency_ms, cost: Tiyin::ZERO, output: None },
        };
        println!("{:<28} {:>7} ms  {:>8} soʻm  {}", row.file, row.latency_ms, row.cost.format_sum(), row.outcome);
        rows.push(row);
    }

    write_reports(&args.out, provider.name(), &rows)?;
    println!("report: {}", args.out.join("report.md").display());
    Ok(())
}

async fn run_one(provider: &dyn TryOnProvider, path: &Path, garment_png: &Bytes, out: &Path, file: &str) -> Result<String> {
    let img = image::load_from_memory(&std::fs::read(path)?).context("not a readable image")?;
    precheck_photo(&img).map_err(|issue| anyhow::anyhow!("pre-check failed: {}", issue.code()))?;
    // Same normalisation as the server: at most 1536 px, JPEG, no metadata.
    let img = if img.width().max(img.height()) > 1536 { img.thumbnail(1536, 1536) } else { img };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&img.to_rgb8())?;
    let result = provider.generate(TryOnRequest { person_jpeg: Bytes::from(jpeg), garment_png: garment_png.clone() }).await?;
    let ext = if result.mime == "image/png" { "png" } else { "jpg" };
    let name = format!("{}.result.{ext}", file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file));
    std::fs::write(out.join(&name), &result.image)?;
    Ok(name)
}

fn write_reports(out: &Path, provider: &str, rows: &[Row]) -> Result<()> {
    let mut csv = String::from("file,outcome,latency_ms,cost_tiyin,output\n");
    for r in rows {
        csv.push_str(&format!("{},{},{},{},{}\n", r.file, r.outcome.replace(',', ";"), r.latency_ms, r.cost.0, r.output.as_deref().unwrap_or("")));
    }
    std::fs::write(out.join("results.csv"), csv)?;

    let ok: Vec<&Row> = rows.iter().filter(|r| r.output.is_some()).collect();
    let mut lat: Vec<u128> = ok.iter().map(|r| r.latency_ms).collect();
    lat.sort_unstable();
    let median = lat.get(lat.len() / 2).copied().unwrap_or(0);
    let total_cost: i64 = rows.iter().map(|r| r.cost.0).sum();
    let mut md = format!(
        "# Try-on evaluation\n\nProvider: **{provider}**\n\n- Photos: {} ({} succeeded, {} failed)\n- Median latency: {median} ms (max {} ms)\n- Total cost: {} soʻm ({} soʻm per successful image)\n\n",
        rows.len(),
        ok.len(),
        rows.len() - ok.len(),
        lat.last().copied().unwrap_or(0),
        Tiyin(total_cost).format_sum(),
        Tiyin(if ok.is_empty() { 0 } else { total_cost / ok.len() as i64 }).format_sum(),
    );
    md.push_str("## Manual quality checklist\n\nOpen each result next to its photo and `garment.png`. Score 1 (bad) to 5 (good).\n\n");
    md.push_str("| Photo | Result | Latency | Garment fidelity (shape, colour) | Design legible and undistorted | Face and body preserved | Artifacts (hands, edges, background) | Notes |\n|---|---|---|---|---|---|---|---|\n");
    for r in rows {
        match &r.output {
            Some(o) => md.push_str(&format!("| {} | {o} | {} ms | | | | | |\n", r.file, r.latency_ms)),
            None => md.push_str(&format!("| {} | failed: {} | {} ms | n/a | n/a | n/a | n/a | |\n", r.file, r.outcome, r.latency_ms)),
        }
    }
    md.push_str("\nAccept a provider when the median of every column is at least 4 and no result scores 1 for face/body preservation.\n");
    std::fs::write(out.join("report.md"), md)?;
    Ok(())
}
