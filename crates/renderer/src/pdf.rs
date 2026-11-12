//! A tiny PDF writer: one page showing one full-page RGB image.
//!
//! The production sheet is laid out as a bitmap with our own text engine
//! (so Uzbek and Cyrillic render with the bundled fonts) and wrapped in a
//! PDF so it prints at exactly A4. Writing these ~40 lines by hand avoids a
//! PDF dependency and keeps the output byte-for-byte deterministic. See
//! ADR 0006 for the trade-off (text is not selectable).

use std::fmt::Write as _;

/// A4 in PostScript points (1/72 inch).
pub const A4_W_PT: f32 = 595.28;
pub const A4_H_PT: f32 = 841.89;

/// Build a single-page PDF of `page_w_pt` x `page_h_pt` points filled with
/// the given 8-bit RGB image (`rgb.len() == width * height * 3`).
pub fn single_image_pdf(rgb: &[u8], width: u32, height: u32, page_w_pt: f32, page_h_pt: f32) -> Vec<u8> {
    assert_eq!(rgb.len(), width as usize * height as usize * 3, "RGB buffer size mismatch");
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(rgb, 6);
    let content = format!("q {page_w_pt:.2} 0 0 {page_h_pt:.2} 0 0 cm /Im0 Do Q\n");

    let mut out: Vec<u8> = Vec::with_capacity(compressed.len() + 1024);
    // The comment line with high bytes tells tools the file is binary.
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    // Byte offset of each object, needed for the cross-reference table.
    let mut offsets: Vec<usize> = Vec::new();

    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w_pt:.2} {page_h_pt:.2}] \
             /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    obj(&mut out, format!("<< /Length {} >>\nstream\n{content}endstream", content.len()).as_bytes());
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB \
         /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        compressed.len()
    )
    .into_bytes();
    image.extend_from_slice(&compressed);
    image.extend_from_slice(b"\nendstream");
    obj(&mut out, &image);

    let xref_at = out.len();
    let mut tail = String::new();
    let _ = write!(tail, "xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for off in &offsets {
        // Each xref entry is exactly 20 bytes: 10-digit offset, generation, flag.
        let _ = writeln!(tail, "{off:010} 00000 n ");
    }
    let _ = write!(tail, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", offsets.len() + 1);
    out.extend_from_slice(tail.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn structure_and_xref_offsets_are_valid() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| (i * 7) as u8).collect();
        let pdf = single_image_pdf(&rgb, 4, 3, A4_W_PT, A4_H_PT);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));

        // startxref must point at the xref keyword.
        let sx = find(&pdf, b"startxref\n").unwrap();
        let num: usize = std::str::from_utf8(&pdf[sx + 10..]).unwrap().lines().next().unwrap().parse().unwrap();
        assert_eq!(&pdf[num..num + 4], b"xref");

        // Every xref entry must point at "N 0 obj".
        let table = std::str::from_utf8(&pdf[num..sx]).unwrap();
        let entries: Vec<&str> = table.lines().skip(3).take(5).collect();
        assert_eq!(entries.len(), 5);
        for (i, e) in entries.iter().enumerate() {
            let off: usize = e[..10].parse().unwrap();
            let want = format!("{} 0 obj", i + 1);
            assert_eq!(&pdf[off..off + want.len()], want.as_bytes());
        }
    }

    #[test]
    fn image_stream_round_trips() {
        let rgb: Vec<u8> = (0..10 * 10 * 3).map(|i| (i % 251) as u8).collect();
        let pdf = single_image_pdf(&rgb, 10, 10, 100.0, 100.0);
        let start = find(&pdf, b"/FlateDecode").unwrap();
        let s = start + find(&pdf[start..], b"stream\n").unwrap() + 7;
        let e = s + find(&pdf[s..], b"\nendstream").unwrap();
        let back = miniz_oxide::inflate::decompress_to_vec_zlib(&pdf[s..e]).unwrap();
        assert_eq!(back, rgb);
    }

    #[test]
    fn output_is_deterministic() {
        let rgb = vec![128u8; 6 * 6 * 3];
        assert_eq!(single_image_pdf(&rgb, 6, 6, 50.0, 50.0), single_image_pdf(&rgb, 6, 6, 50.0, 50.0));
    }
}
