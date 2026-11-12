//! Background removal for uploaded artwork.
//!
//! The built-in implementation is a real, local algorithm rather than a
//! stub: it removes a uniform background (the usual case for logos and
//! clip-art on white) by flood-filling from the image border. It will not
//! cut a person out of a photo; plug in a hosted matting model behind the
//! same trait for that.

use std::collections::VecDeque;

use async_trait::async_trait;
use image::RgbaImage;

#[derive(Debug, thiserror::Error)]
#[error("background removal failed: {0}")]
pub struct BgError(pub String);

#[async_trait]
pub trait BackgroundRemover: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn remove(&self, image: RgbaImage) -> Result<RgbaImage, BgError>;
}

/// Removes pixels connected to the border that are close to the border's
/// dominant colour.
pub struct KeyColorRemover {
    /// Max per-channel distance (0-255) still treated as background.
    pub tolerance: u8,
}

impl Default for KeyColorRemover {
    fn default() -> Self {
        KeyColorRemover { tolerance: 28 }
    }
}

#[async_trait]
impl BackgroundRemover for KeyColorRemover {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn remove(&self, image: RgbaImage) -> Result<RgbaImage, BgError> {
        let tol = self.tolerance;
        // CPU-bound work must not block the async runtime's worker threads.
        tokio::task::spawn_blocking(move || key_out(image, tol)).await.map_err(|e| BgError(e.to_string()))
    }
}

fn close(a: [u8; 4], b: [u8; 3], tol: u8) -> bool {
    a[0].abs_diff(b[0]) <= tol && a[1].abs_diff(b[1]) <= tol && a[2].abs_diff(b[2]) <= tol
}

/// Synchronous core, exposed for tests and the evaluation harness.
pub fn key_out(mut img: RgbaImage, tol: u8) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w < 3 || h < 3 {
        return img;
    }
    // Background colour = average of the four corners' most common one.
    let corners = [img.get_pixel(0, 0).0, img.get_pixel(w - 1, 0).0, img.get_pixel(0, h - 1).0, img.get_pixel(w - 1, h - 1).0];
    let mut best = corners[0];
    let mut best_votes = 0;
    for c in corners {
        let votes = corners.iter().filter(|o| close(**o, [c[0], c[1], c[2]], tol)).count();
        if votes > best_votes {
            best = c;
            best_votes = votes;
        }
    }
    // Corners disagree or are already transparent: nothing sensible to do.
    if best_votes < 3 || best[3] < 128 {
        return img;
    }
    let key = [best[0], best[1], best[2]];

    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let mut seen = vec![false; (w * h) as usize];
    let mut queue: VecDeque<(u32, u32)> = VecDeque::new();
    let push = |x: u32, y: u32, seen: &mut Vec<bool>, queue: &mut VecDeque<(u32, u32)>, img: &RgbaImage| {
        if !seen[idx(x, y)] && close(img.get_pixel(x, y).0, key, tol) {
            seen[idx(x, y)] = true;
            queue.push_back((x, y));
        }
    };
    for x in 0..w {
        push(x, 0, &mut seen, &mut queue, &img);
        push(x, h - 1, &mut seen, &mut queue, &img);
    }
    for y in 0..h {
        push(0, y, &mut seen, &mut queue, &img);
        push(w - 1, y, &mut seen, &mut queue, &img);
    }
    while let Some((x, y)) = queue.pop_front() {
        if x > 0 {
            push(x - 1, y, &mut seen, &mut queue, &img);
        }
        if x + 1 < w {
            push(x + 1, y, &mut seen, &mut queue, &img);
        }
        if y > 0 {
            push(x, y - 1, &mut seen, &mut queue, &img);
        }
        if y + 1 < h {
            push(x, y + 1, &mut seen, &mut queue, &img);
        }
    }
    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] {
                img.get_pixel_mut(x, y).0[3] = 0;
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    /// White canvas with a red square that has a white hole in the middle.
    fn logo() -> RgbaImage {
        RgbaImage::from_fn(40, 40, |x, y| {
            let in_square = (10..30).contains(&x) && (10..30).contains(&y);
            let in_hole = (18..22).contains(&x) && (18..22).contains(&y);
            if in_square && !in_hole {
                image::Rgba([220, 20, 30, 255])
            } else {
                image::Rgba([250, 252, 255, 255])
            }
        })
    }

    #[tokio::test]
    async fn removes_border_connected_background_only() {
        let out = KeyColorRemover::default().remove(logo()).await.unwrap();
        assert_eq!(out.get_pixel(0, 0).0[3], 0, "corner must be transparent");
        assert_eq!(out.get_pixel(5, 20).0[3], 0);
        assert_eq!(out.get_pixel(15, 15).0, [220, 20, 30, 255], "artwork must be untouched");
        // The enclosed white hole is not connected to the border: kept.
        assert_eq!(out.get_pixel(20, 20).0[3], 255);
    }

    #[test]
    fn leaves_photos_and_transparent_images_alone() {
        // Four different corners: no dominant background.
        let photo = RgbaImage::from_fn(20, 20, |x, y| image::Rgba([(x * 12) as u8, (y * 12) as u8, 90, 255]));
        assert_eq!(key_out(photo.clone(), 28), photo);
        let transparent = RgbaImage::from_pixel(20, 20, image::Rgba([255, 255, 255, 0]));
        assert_eq!(key_out(transparent.clone(), 28), transparent);
    }
}
