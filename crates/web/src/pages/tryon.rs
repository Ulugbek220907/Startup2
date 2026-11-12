//! Try-on: live camera (on-device), photo (server) and a plain preview.
//!
//! Every path here is optional for the customer: whatever fails, the
//! "order" button at the bottom keeps working.

use std::cell::RefCell;
use std::rc::Rc;

use design::garment_art::{tryon_anchors, ART_H, ART_W};
use design::tryon::{build_mesh, BodyPose, Landmark, OneEuro, PoseIssue, LM_COUNT};
use design::{Color, Side};
use domain::api::{DeletedDto, TryOnDto, TryOnState};
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, HtmlImageElement, HtmlVideoElement};

use crate::app::TopBar;
use crate::draw::{self, MockupOpts, View};
use crate::icons;
use crate::router::{use_router, Route};
use crate::state::{ctx, device_id, t, tf, Ctx, Fonts};
use crate::tryon_gl::Renderer;
use crate::{api, platform};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Live,
    Photo,
    Plain,
}

#[derive(Clone, Copy, PartialEq)]
enum LiveStatus {
    Idle,
    /// Model download progress, 0..=100.
    Loading(u32),
    Running,
    Denied,
    Unsupported,
}

/// Paint the whole-garment mockup of the current draft onto a canvas.
fn paint_draft(ctx: Ctx, canvas: &HtmlCanvasElement, side: Side, css_w: f32, background: Option<&'static str>) -> bool {
    let Some(garment) = ctx.garment() else { return false };
    ctx.draft.with(|d| {
        let Some(doc) = d.doc.as_ref() else { return false };
        let color = garment.color(&doc.color).and_then(|c| Color::parse(&c.hex)).unwrap_or(Color::BLACK);
        let scale = draw::fit_canvas(canvas, css_w, css_w * View::FULL.aspect());
        let Some(c) = draw::context_2d(canvas) else { return false };
        draw::draw_mockup(
            &c,
            css_w,
            scale,
            &View::FULL,
            garment.kind,
            side,
            color,
            garment.area(side),
            doc.layers(side),
            &MockupOpts { show_area: false, background },
        );
        // The garment image loads asynchronously; report whether it was there.
        draw::image(&draw::garment_url(garment.kind, side, color)).is_some()
    })
}

#[component]
pub fn TryOnPage() -> impl IntoView {
    let ctx = ctx();
    let router = use_router();
    let mode = RwSignal::new(Mode::Live);
    let has_design = move || ctx.draft.with(|d| d.doc.as_ref().is_some_and(|doc| !doc.is_empty()));
    if let Some(doc) = ctx.draft.with_untracked(|d| d.doc.clone()) {
        crate::state::ensure_doc_fonts(&doc);
    }

    view! {
        <TopBar title=Signal::derive(move || t("tryon.title").to_string())/>
        <main class="page has-bar">
            <Show
                when=has_design
                fallback=move || {
                    view! {
                        <div class="note">{move || t("order.empty_design")}</div>
                        <button class="btn btn-primary" on:click=move |_| router.go(Route::Home)>{move || t("home.start")}</button>
                    }
                }
            >
                <div class="seg">
                    <button class:on=move || mode.get() == Mode::Live on:click=move |_| mode.set(Mode::Live)>{move || t("tryon.live")}</button>
                    <button class:on=move || mode.get() == Mode::Photo on:click=move |_| mode.set(Mode::Photo)>{move || t("tryon.photo")}</button>
                    <button class:on=move || mode.get() == Mode::Plain on:click=move |_| mode.set(Mode::Plain)>{move || t("tryon.static")}</button>
                </div>
                // Switching tabs drops the previous one, which stops its camera.
                {move || match mode.get() {
                    Mode::Live => view! { <LiveTryOn mode=mode/> }.into_any(),
                    Mode::Photo => view! { <PhotoTryOn/> }.into_any(),
                    Mode::Plain => view! { <PlainPreview/> }.into_any(),
                }}
            </Show>
        </main>
        <div class="sticky-bar">
            <button class="btn btn-primary btn-block" on:click=move |_| router.go(Route::Order)>{icons::bag()}{move || t("editor.order")}</button>
        </div>
    }
}

// ------------------------------------------------------------------ plain

#[component]
fn PlainPreview() -> impl IntoView {
    let ctx = ctx();
    let side = RwSignal::new(Side::Front);
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    Effect::new(move |_| {
        ctx.redraw.track();
        let s = side.get();
        if let Some(canvas) = canvas_ref.get() {
            let w = canvas.parent_element().map(|p| p.client_width() as f32).unwrap_or(320.0);
            paint_draft(ctx, &canvas, s, w, None);
        }
    });
    view! {
        <div class="viewer plain" style="aspect-ratio:10/11">
            <span class="badge">{move || t("tryon.preview_label")}</span>
            <canvas node_ref=canvas_ref></canvas>
        </div>
        <div class="seg">
            <button class:on=move || side.get() == Side::Front on:click=move |_| side.set(Side::Front)>{move || t("common.front")}</button>
            <button class:on=move || side.get() == Side::Back on:click=move |_| side.set(Side::Back)>{move || t("common.back_side")}</button>
        </div>
    }
}

// ------------------------------------------------------------------- live

/// Mutable state of the render loop. Lives in an `Rc<RefCell<..>>` because
/// the animation-frame callback and the UI both touch it, on one thread.
struct Live {
    renderer: Option<Renderer>,
    /// Off-screen canvas holding the garment artwork with the design.
    garment_canvas: Option<HtmlCanvasElement>,
    garment_dirty: bool,
    filters: Vec<OneEuro>,
    mirrored: bool,
    frames: u32,
    window_start: f64,
    slow_windows: u32,
}

fn pose_key(issue: PoseIssue) -> &'static str {
    match issue {
        PoseIssue::NoPerson => "tryon.no_person",
        PoseIssue::StepBack => "tryon.tip_step_back",
        PoseIssue::FaceCamera => "tryon.tip_face",
    }
}

/// Smooth the eight joints with One-Euro filters (16 scalar channels).
fn smooth(pose: BodyPose, filters: &mut [OneEuro], t: f64) -> BodyPose {
    let mut pts = [pose.shoulder_l, pose.shoulder_r, pose.hip_l, pose.hip_r, pose.elbow_l, pose.elbow_r, pose.wrist_l, pose.wrist_r];
    for (i, p) in pts.iter_mut().enumerate() {
        p[0] = filters[i * 2].filter(p[0], t);
        p[1] = filters[i * 2 + 1].filter(p[1], t);
    }
    BodyPose {
        shoulder_l: pts[0],
        shoulder_r: pts[1],
        hip_l: pts[2],
        hip_r: pts[3],
        elbow_l: pts[4],
        elbow_r: pts[5],
        wrist_l: pts[6],
        wrist_r: pts[7],
        ..pose
    }
}

#[component]
fn LiveTryOn(mode: RwSignal<Mode>) -> impl IntoView {
    let ctx = ctx();
    let status = RwSignal::new(LiveStatus::Idle);
    let tip = RwSignal::new(None::<&'static str>);
    let slow = RwSignal::new(false);
    let facing_user = RwSignal::new(true);
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    // `new_local` keeps this non-`Send` state (WebGL handles) in Leptos'
    // single-threaded arena and gives back a `Copy` handle that closures can
    // capture freely.
    let live = StoredValue::new_local(Live {
        renderer: None,
        garment_canvas: None,
        garment_dirty: true,
        filters: (0..16).map(|_| OneEuro::new(1.2, 0.02)).collect(),
        mirrored: true,
        frames: 0,
        window_start: 0.0,
        slow_windows: 0,
    });

    let stop = StoredValue::new(false);
    // Re-upload the garment texture when the design or an image changes.
    let dirty = StoredValue::new(true);
    Effect::new(move |_| {
        ctx.redraw.track();
        ctx.draft.track();
        dirty.set_value(true);
    });

    // One frame of the loop. Returns false to stop.
    let frame = move |now_ms: f64| -> bool {
        if stop.try_get_value().unwrap_or(true) {
            return false;
        }
        live.try_update_value(|st| {
            if dirty.get_value() {
                dirty.set_value(false);
                st.garment_dirty = true;
            }
            let (Some(video), Some(canvas)) = (video_ref.get_untracked(), canvas_ref.get_untracked()) else { return false };
            let video: HtmlVideoElement = video;
            let (vw, vh) = (video.video_width(), video.video_height());
            if vw == 0 || vh == 0 || video.ready_state() < 2 {
                return true; // camera still starting
            }
            if canvas.width() != vw || canvas.height() != vh {
                canvas.set_width(vw);
                canvas.set_height(vh);
            }
            if st.renderer.is_none() {
                match Renderer::new(&canvas) {
                    Ok(r) => st.renderer = Some(r),
                    Err(_) => {
                        status.set(LiveStatus::Unsupported);
                        return false;
                    }
                }
            }
            // Refresh the garment texture when needed (and once its image has loaded).
            if st.garment_dirty {
                if st.garment_canvas.is_none() {
                    st.garment_canvas = web_sys::window()
                        .and_then(|w| w.document())
                        .and_then(|d| d.create_element("canvas").ok())
                        .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok());
                }
                if let Some(gc) = st.garment_canvas.clone() {
                    // 640 px wide is sharp enough on a phone and cheap to upload.
                    gc.set_width(640);
                    gc.set_height((640.0 * ART_H / ART_W) as u32);
                    let ready = paint_draft_fixed(ctx, &gc, 640.0);
                    if let Some(r) = st.renderer.as_mut() {
                        r.set_garment(&gc);
                    }
                    st.garment_dirty = !ready;
                }
            }

            let mirrored = st.mirrored;
            let t = now_ms / 1000.0;
            let found = platform::pose_detect(&video, now_ms);
            let pose = if found > 0 {
                let raw = platform::pose_landmarks().to_vec();
                let lm: Vec<Landmark> = (0..LM_COUNT).map(|i| Landmark { x: raw[i * 3], y: raw[i * 3 + 1], visibility: raw[i * 3 + 2] }).collect();
                BodyPose::from_landmarks(&lm, vw as f32, vh as f32, mirrored)
            } else {
                Err(PoseIssue::NoPerson)
            };
            let kind = ctx.garment().map(|g| g.kind);
            match (pose, kind) {
                (Ok(pose), Some(kind)) => {
                    let pose = smooth(pose, &mut st.filters, t);
                    let mesh = build_mesh(&tryon_anchors(kind), &pose, 20, 22);
                    // The mask is only needed (and only fetched) while a
                    // forearm crosses the torso.
                    let arms_in_front = pose.forearm_front_l || pose.forearm_front_r;
                    platform::pose_want_mask(arms_in_front);
                    if let Some(r) = st.renderer.as_mut() {
                        if arms_in_front {
                            let (mw, mh) = (platform::pose_mask_width(), platform::pose_mask_height());
                            r.set_mask(&platform::pose_mask().to_vec(), mw, mh);
                        }
                        r.draw(&video, vw as f32, vh as f32, mirrored, Some((&mesh, &pose)));
                    }
                    if tip.get_untracked().is_some() {
                        tip.set(None);
                    }
                }
                (Err(issue), _) => {
                    // Forget the smoothing history so the garment does not
                    // glide in from its old position when the person returns.
                    st.filters.iter_mut().for_each(OneEuro::reset);
                    if let Some(r) = st.renderer.as_mut() {
                        r.draw(&video, vw as f32, vh as f32, mirrored, None);
                    }
                    let key = pose_key(issue);
                    if tip.get_untracked() != Some(key) {
                        tip.set(Some(key));
                    }
                }
                _ => {}
            }

            // Frame-rate watch: three 2-second windows under 12 fps in a row
            // mean this device is too slow for a good live preview.
            st.frames += 1;
            if st.window_start == 0.0 {
                st.window_start = now_ms;
            } else if now_ms - st.window_start >= 2000.0 {
                let fps = st.frames as f64 * 1000.0 / (now_ms - st.window_start);
                st.slow_windows = if fps < 12.0 { st.slow_windows + 1 } else { 0 };
                if st.slow_windows >= 3 && !slow.get_untracked() {
                    slow.set(true);
                }
                st.frames = 0;
                st.window_start = now_ms;
            }
            true
        })
        .unwrap_or(false)
    };

    // requestAnimationFrame loop. The closure re-schedules itself through a
    // shared handle; it is dropped when a frame returns false.
    let start_loop = {
        move || {
            let handle: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
            let again = handle.clone();
            *handle.borrow_mut() = Some(Closure::new(move |now: f64| {
                if frame(now) {
                    if let (Some(w), Some(cb)) = (web_sys::window(), again.borrow().as_ref()) {
                        let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
                    }
                } else {
                    // Break the reference cycle so the closure is freed.
                    again.borrow_mut().take();
                }
            }));
            let first = handle.borrow();
            if let (Some(w), Some(cb)) = (web_sys::window(), first.as_ref()) {
                let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
            };
        }
    };

    let start = {
        move || {
            let Some(video) = video_ref.get_untracked() else { return };
            let facing = if facing_user.get_untracked() { "user" } else { "environment" };
            let already_running = status.get_untracked() == LiveStatus::Running;
            live.update_value(|st| st.mirrored = facing_user.get_untracked());
            if !already_running {
                status.set(LiveStatus::Loading(0));
            }
            leptos::task::spawn_local(async move {
                match platform::await_string(platform::start_camera(&video, facing)).await.as_str() {
                    "ok" => {}
                    "denied" => return status.set(LiveStatus::Denied),
                    _ => return status.set(LiveStatus::Unsupported),
                }
                if already_running {
                    return; // camera switched; the loop is already going
                }
                let progress = Closure::<dyn FnMut(f64)>::new(move |p: f64| status.set(LiveStatus::Loading(p.clamp(0.0, 100.0) as u32)));
                let ok = wasm_bindgen_futures::JsFuture::from(platform::pose_init(&progress)).await.ok().and_then(|v| v.as_bool()).unwrap_or(false);
                drop(progress);
                if !ok {
                    platform::stop_camera(&video);
                    return status.set(LiveStatus::Unsupported);
                }
                status.set(LiveStatus::Running);
                start_loop();
            });
        }
    };

    // Leaving the screen: stop the loop and release the camera. The flag is
    // a `StoredValue` (not part of `Live`) because clean-up callbacks must
    // be `Send`, which an `Rc` is not.
    on_cleanup(move || {
        stop.set_value(true);
        if let Some(v) = video_ref.get_untracked() {
            platform::stop_camera(&v);
        }
    });

    let snapshot = move |_| {
        let Some(canvas) = canvas_ref.get_untracked() else { return };
        platform::haptic("light");
        leptos::task::spawn_local(async move {
            let blob = wasm_bindgen_futures::JsFuture::from(platform::canvas_blob(&canvas, "image/jpeg", 0.9))
                .await
                .ok()
                .and_then(|b| b.dyn_into::<web_sys::Blob>().ok());
            if let Some(blob) = blob {
                let brand = ctx.catalog.with_untracked(|c| c.as_ref().map(|c| c.brand.clone()).unwrap_or_default());
                platform::await_string(platform::share(&brand, None, Some(&blob), "tryon.jpg")).await;
            }
        });
    };

    let start_click = move |_| start();
    let switch_camera = move |_| {
        facing_user.update(|f| *f = !*f);
        start();
    };

    view! {
        <div class="viewer">
            <span class="badge">{move || t("tryon.preview_label")}</span>
            // The video element only feeds the model and the WebGL texture.
            <video node_ref=video_ref playsinline=true muted=true style="opacity:0"></video>
            <canvas node_ref=canvas_ref></canvas>
            {move || match status.get() {
                LiveStatus::Running => tip.get().map(|key| view! { <div class="tip">{move || t(key)}</div> }.into_any()).unwrap_or_else(|| ().into_any()),
                LiveStatus::Idle => {
                    view! {
                        <div class="overlay">
                            <ul class="tips" style="text-align:left">
                                <li>{move || t("tryon.tip_step_back")}</li>
                                <li>{move || t("tryon.tip_torso")}</li>
                                <li>{move || t("tryon.tip_light")}</li>
                            </ul>
                            <button class="btn btn-primary" on:click=start_click>{icons::camera()}{move || t("tryon.start_camera")}</button>
                        </div>
                    }
                        .into_any()
                }
                LiveStatus::Loading(pct) => view! {
                    <div class="overlay">
                        <div>{move || tf("tryon.loading_model", &[("pct", &pct.to_string())])}</div>
                        <div class="progress"><i style=format!("width:{pct}%")></i></div>
                    </div>
                }
                    .into_any(),
                LiveStatus::Denied | LiveStatus::Unsupported => {
                    let key = if status.get_untracked() == LiveStatus::Denied { "tryon.camera_denied" } else { "tryon.unsupported" };
                    view! {
                        <div class="overlay">
                            <div>{move || t(key)}</div>
                            <button class="btn btn-primary" on:click=move |_| mode.set(Mode::Photo)>{move || t("tryon.photo")}</button>
                            <button class="btn btn-ghost" on:click=move |_| mode.set(Mode::Plain)>{move || t("tryon.static")}</button>
                        </div>
                    }
                        .into_any()
                }
            }}
        </div>
        <Show when=move || slow.get()>
            <div class="note warn">{move || t("tryon.slow")}</div>
            <button class="btn btn-ghost" on:click=move |_| mode.set(Mode::Photo)>{move || t("tryon.photo")}</button>
        </Show>
        <Show when=move || status.get() == LiveStatus::Running>
            <div class="row">
                <button class="btn btn-ghost grow" on:click=snapshot>{icons::camera()}{move || t("tryon.snapshot")}</button>
                <button class="icon-btn" style="background:var(--surface);border:1px solid var(--line)" aria-label=move || t("tryon.switch_camera") on:click=switch_camera>
                    {icons::swap()}
                </button>
            </div>
        </Show>
        <GarmentColorRow/>
    }
}

/// Paint the front mockup at a fixed pixel width (no device-pixel scaling),
/// for use as a WebGL texture.
fn paint_draft_fixed(ctx: Ctx, canvas: &HtmlCanvasElement, width: f32) -> bool {
    let Some(garment) = ctx.garment() else { return false };
    ctx.draft.with_untracked(|d| {
        let Some(doc) = d.doc.as_ref() else { return false };
        let color = garment.color(&doc.color).and_then(|c| Color::parse(&c.hex)).unwrap_or(Color::BLACK);
        let Some(c) = draw::context_2d(canvas) else { return false };
        draw::draw_mockup(
            &c,
            width,
            1.0,
            &View::FULL,
            garment.kind,
            Side::Front,
            color,
            garment.area(Side::Front),
            doc.layers(Side::Front),
            &MockupOpts { show_area: false, background: None },
        );
        let assets_ready = doc.front.iter().all(|l| match &l.kind {
            design::LayerKind::Image(i) => draw::image(&draw::asset_preview_url(&i.asset)).is_some(),
            design::LayerKind::Text(t) => design::fonts::FontSource::font_data(&Fonts, t.font).is_some(),
            _ => true,
        });
        assets_ready && draw::image(&draw::garment_url(garment.kind, Side::Front, color)).is_some()
    })
}

/// Garment colour switcher shared by the try-on tabs.
#[component]
fn GarmentColorRow() -> impl IntoView {
    let ctx = ctx();
    let current = move || ctx.draft.with(|d| d.doc.as_ref().map(|doc| doc.color.clone()));
    view! {
        <div class="swatches">
            {move || {
                ctx.garment()
                    .map(|g| g.colors)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| {
                        let (slug, slug2) = (c.slug.clone(), c.slug.clone());
                        let name = c.name.clone();
                        view! {
                            <button
                                class="swatch"
                                style=format!("background:{}", c.hex)
                                class:on=move || current().as_deref() == Some(slug.as_str())
                                aria-label=move || name.get(ctx.lang.get()).to_string()
                                on:click=move |_| {
                                    let s = slug2.clone();
                                    ctx.draft.update(|d| {
                                        if let Some(doc) = d.doc.as_mut() {
                                            doc.color = s;
                                        }
                                    });
                                    ctx.persist_draft();
                                }
                            ></button>
                        }
                    })
                    .collect_view()
            }}
        </div>
    }
}

// ------------------------------------------------------------------ photo

#[derive(Clone, PartialEq)]
enum PhotoStatus {
    Consent,
    Checking,
    Generating,
    Done(TryOnDto),
    /// Translation key of what went wrong.
    Failed(String),
}

/// Client-side checks before upload: resolution, brightness, and (when the
/// pose model can run here) exactly one person with a visible torso.
async fn precheck(file: &web_sys::File) -> Result<(), &'static str> {
    let Ok(url) = web_sys::Url::create_object_url_with_blob(file) else { return Ok(()) };
    let img = HtmlImageElement::new().map_err(|_| "error.upload_invalid")?;
    img.set_src(&url);
    let decoded = wasm_bindgen_futures::JsFuture::from(img.decode()).await;
    let result = async {
        if decoded.is_err() {
            return Err("error.upload_invalid");
        }
        if img.natural_width().min(img.natural_height()) < 480 {
            return Err("error.tryon_photo_small");
        }
        // Mean brightness of a 48x48 thumbnail.
        if let Some(canvas) =
            web_sys::window().and_then(|w| w.document()).and_then(|d| d.create_element("canvas").ok()).and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
        {
            canvas.set_width(48);
            canvas.set_height(48);
            if let Some(c) = draw::context_2d(&canvas) {
                let _ = c.draw_image_with_html_image_element_and_dw_and_dh(&img, 0.0, 0.0, 48.0, 48.0);
                if let Ok(data) = c.get_image_data(0.0, 0.0, 48.0, 48.0) {
                    let px = data.data();
                    let sum: u32 = px.chunks(4).map(|p| (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000).sum();
                    if sum / (48 * 48) < 45 {
                        return Err("error.tryon_photo_dark");
                    }
                }
            }
        }
        // Person check. If the model cannot load on this device the check is
        // skipped; the server still validates the photo.
        let noop = Closure::<dyn FnMut(f64)>::new(|_| {});
        let loaded = wasm_bindgen_futures::JsFuture::from(platform::pose_init(&noop)).await.ok().and_then(|v| v.as_bool()).unwrap_or(false);
        if loaded {
            let verdict = wasm_bindgen_futures::JsFuture::from(platform::pose_check_image(&img)).await.ok().and_then(|v| v.as_f64()).unwrap_or(-1.0) as i32;
            match verdict {
                0 => return Err("error.tryon_no_person"),
                2 => return Err("error.tryon_many_people"),
                3 => return Err("error.tryon_no_torso"),
                _ => {}
            }
        }
        Ok(())
    }
    .await;
    let _ = web_sys::Url::revoke_object_url(&url);
    result
}

#[component]
fn PhotoTryOn() -> impl IntoView {
    let ctx = ctx();
    let status = RwSignal::new(PhotoStatus::Consent);
    let agreed = RwSignal::new(false);
    let file_ref = NodeRef::<leptos::html::Input>::new();
    let hours = move || ctx.catalog.with(|c| c.as_ref().map(|c| c.tryon_ttl_hours).unwrap_or(24)).to_string();
    // Set when the screen is left, so polling stops.
    let gone = StoredValue::new(false);
    on_cleanup(move || gone.set_value(true));

    let on_file = move |_| {
        let Some(input) = file_ref.get_untracked() else { return };
        let Some(file) = input.files().and_then(|f| f.get(0)) else { return };
        input.set_value("");
        if !agreed.get_untracked() {
            return;
        }
        status.set(PhotoStatus::Checking);
        leptos::task::spawn_local(async move {
            if let Err(key) = precheck(&file).await {
                return status.set(PhotoStatus::Failed(key.to_string()));
            }
            status.set(PhotoStatus::Generating);
            let fail = |e: api::ApiFail| status.set(PhotoStatus::Failed(e.key()));
            let slug = match ctx.ensure_saved().await {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let Some(bytes) = api::read_file(&file).await else { return status.set(PhotoStatus::Failed("error.upload_invalid".into())) };
            let headers = [("x-device-id", device_id()), ("x-consent", "yes".to_string())];
            let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let mut dto: TryOnDto = match api::post_bytes(&format!("/api/tryon?design={slug}&side=front"), &bytes, &headers).await {
                Ok(d) => d,
                Err(e) => return fail(e),
            };
            // Poll until the worker finishes (about 2 minutes at most).
            for _ in 0..90 {
                match dto.status {
                    TryOnState::Done => {
                        platform::haptic("success");
                        return status.set(PhotoStatus::Done(dto));
                    }
                    TryOnState::Failed | TryOnState::Deleted => return status.set(PhotoStatus::Failed("tryon.failed".into())),
                    TryOnState::Queued | TryOnState::Running => {}
                }
                sleep_ms(1500).await;
                if gone.try_get_value().unwrap_or(true) {
                    return;
                }
                match api::get::<TryOnDto>(&format!("/api/tryon/{}", dto.id)).await {
                    Ok(d) => dto = d,
                    Err(e) if e.status == 0 => {} // brief network loss: keep polling
                    Err(e) => return fail(e),
                }
            }
            status.set(PhotoStatus::Failed("tryon.failed".into()));
        });
    };

    let delete_now = move |id: String| {
        leptos::task::spawn_local(async move {
            match api::delete::<DeletedDto>(&format!("/api/tryon/{id}")).await {
                Ok(_) => {
                    ctx.show_toast(t("tryon.deleted"), false);
                    status.set(PhotoStatus::Consent);
                }
                Err(e) => ctx.show_error(&e),
            }
        });
    };
    let pick = move |_| {
        if let Some(i) = file_ref.get_untracked() {
            i.click();
        }
    };

    view! {
        // `capture` is deliberately not set: the customer may take a new
        // photo or choose an existing one.
        <input node_ref=file_ref type="file" accept="image/jpeg,image/png,image/webp" hidden on:change=on_file/>
        {move || match status.get() {
            PhotoStatus::Consent => view! {
                <div class="card">
                    <h3 class="section">{move || t("tryon.consent_title")}</h3>
                    <ul class="tips">
                        <li>{move || t("tryon.consent_1")}</li>
                        <li>{move || tf("tryon.consent_2", &[("hours", &hours())])}</li>
                        <li>{move || t("tryon.consent_3")}</li>
                    </ul>
                    <label class="check">
                        <input type="checkbox" prop:checked=move || agreed.get() on:change=move |ev| agreed.set(event_target_checked(&ev))/>
                        <span>{move || t("tryon.consent_check")}</span>
                    </label>
                    <button class="btn btn-primary" disabled=move || !agreed.get() on:click=pick>{icons::image()}{move || t("tryon.choose_photo")}</button>
                </div>
                <ul class="tips">
                    <li>{move || t("tryon.tip_torso")}</li>
                    <li>{move || t("tryon.tip_light")}</li>
                    <li>{move || t("tryon.tip_face")}</li>
                </ul>
            }
                .into_any(),
            PhotoStatus::Checking | PhotoStatus::Generating => view! {
                <div class="viewer">
                    <div class="overlay">
                        <div class="boot-mark"></div>
                        <div>{move || t("tryon.generating")}</div>
                    </div>
                </div>
            }
                .into_any(),
            PhotoStatus::Done(dto) => {
                let src = dto.result_url.as_deref().map(api::url).unwrap_or_default();
                let id = dto.id.clone();
                view! {
                    <div class="viewer">
                        <span class="badge">{move || t("tryon.preview_label")}</span>
                        <img src=src alt=""/>
                    </div>
                    <div class="row">
                        <button class="btn btn-ghost grow" on:click=move |_| status.set(PhotoStatus::Consent)>{move || t("tryon.another_photo")}</button>
                        <button class="btn btn-danger grow" on:click=move |_| delete_now(id.clone())>{icons::trash()}{move || t("tryon.delete_now")}</button>
                    </div>
                }
                    .into_any()
            }
            PhotoStatus::Failed(key) => view! {
                <div class="note bad">{move || t(&key)}</div>
                <p class="muted">{move || t("tryon.failed")}</p>
                <button class="btn btn-ghost" on:click=move |_| status.set(PhotoStatus::Consent)>{move || t("tryon.another_photo")}</button>
            }
                .into_any(),
        }}
    }
}

/// Await a timer.
pub async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
