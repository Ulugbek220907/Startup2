# ADR 0009: Live try-on with MediaPipe pose + a WebGL mesh warp

**Status:** accepted

## Decision
- **Model:** MediaPipe Pose Landmarker (lite, float16) through
  `@mediapipe/tasks-vision` 1.2.3. One model gives 33 body landmarks and a
  person segmentation mask. It is self-hosted under `/vendor/mediapipe`
  (downloaded and checksum-verified at build time), loaded only when the
  customer starts live try-on, and cached by the service worker. Camera
  frames never leave the device.
- **Warp:** the flat garment artwork is drawn on a 20x22 vertex grid. Each
  vertex blends three transforms (linear blend skinning): the torso quad
  through shoulders and hips, the upper arm and, for long sleeves, the
  forearm. The math is pure Rust in `design::tryon` with native unit tests.
- **Smoothing:** One-Euro filters on the eight joints.
- **Occlusion:** when a wrist is inside the torso, the camera frame is drawn
  again over the garment inside a capsule around that forearm, limited to
  where the mask says "person". The mask is read back only in that case.
- **Fallbacks:** camera refused, model cannot load, or three consecutive
  2-second windows under 12 fps each lead to an offer of photo try-on or
  the plain preview. Ordering never depends on try-on.

## What is and is not verified
- Verified in headless Chromium with a fake camera fed MediaPipe's sample
  photo: model loads, pose is found, the T-shirt is warped onto the torso
  with sleeves following the arms, and the slow-device fallback appears
  (software rendering there runs at about 4 fps).
- **Not verified:** frame rate on real phones. The 20 fps target on a
  mid-range Android device is untested and must be checked with
  `docs/MANUAL_TESTS.md`.
- **Known miss:** "model load under 5 s on 4G" is not achievable on a first
  visit. The runtime is about 4.2 MB compressed and the model 5.8 MB; on a
  typical 4G link that is 8-15 s once, then near-instant from cache. A
  progress bar is shown. Reducing this would need a smaller custom model.
- v1 handles a front-facing standing pose. Side and back views are out of
  scope and will look wrong.
