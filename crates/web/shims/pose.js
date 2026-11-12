// On-device pose + person segmentation (MediaPipe Pose Landmarker).
//
// WHY THIS FILE EXISTS: MediaPipe ships as a JavaScript module with its own
// WASM runtime and has no Rust binding. This shim loads it lazily (only
// when the customer opens the try-on screen), reports download progress,
// and hands plain typed arrays to Rust. Camera frames are processed in the
// browser and are never uploaded.

const VENDOR = "/vendor/mediapipe";
const MODEL = `${VENDOR}/pose_landmarker_lite.task`;

let landmarker = null;      // VIDEO mode, one person
let imageLandmarker = null; // IMAGE mode, up to two people (photo pre-check)
let fileset = null;
let modelBytes = null;
let loading = null;

// 33 landmarks x (x, y, visibility), reused every frame to avoid garbage.
const landmarks = new Float32Array(33 * 3);
let mask = new Uint8Array(0);
let maskW = 0;
let maskH = 0;
// Reading the mask back from the GPU costs a few milliseconds per frame, so
// it is only done while Rust says an arm is in front of the body.
let wantMask = false;

async function fetchWithProgress(url, onProgress, from, to) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`download failed: ${res.status}`);
  // With gzip/brotli the header is the compressed size while we count
  // decompressed bytes, so progress is clamped rather than trusted.
  const total = Number(res.headers.get("content-length")) || 0;
  if (!res.body) {
    const buf = new Uint8Array(await res.arrayBuffer());
    onProgress(to);
    return buf;
  }
  const reader = res.body.getReader();
  const chunks = [];
  let got = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    got += value.length;
    if (total) onProgress(from + (to - from) * Math.min(0.98, got / total));
  }
  const out = new Uint8Array(got);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  onProgress(to);
  return out;
}

async function load(onProgress) {
  // The runtime is ~4 MB compressed and the model ~6 MB; both are cached by
  // the service worker after the first visit.
  onProgress(2);
  const vision = await import(/* webpackIgnore: true */ `${VENDOR}/vision_bundle.mjs`);
  onProgress(10);
  // Warm the cache for the big WASM file so progress reflects it.
  await fetchWithProgress(`${VENDOR}/wasm/vision_wasm_internal.wasm`, onProgress, 10, 55).catch(() => null);
  fileset = await vision.FilesetResolver.forVisionTasks(`${VENDOR}/wasm`);
  modelBytes = await fetchWithProgress(MODEL, onProgress, 55, 92);

  const make = (delegate) => vision.PoseLandmarker.createFromOptions(fileset, {
    baseOptions: { modelAssetBuffer: modelBytes, delegate },
    runningMode: "VIDEO",
    numPoses: 1,
    minPoseDetectionConfidence: 0.5,
    minPosePresenceConfidence: 0.5,
    minTrackingConfidence: 0.5,
    outputSegmentationMasks: true,
  });
  try {
    landmarker = await make("GPU");
  } catch (_) {
    // Some devices have no usable WebGL for ML; the CPU path still works.
    landmarker = await make("CPU");
  }
  loading = vision;
  onProgress(100);
}

/** Load the model. Resolves true when ready, false if this device cannot. */
export async function poseInit(onProgress) {
  if (landmarker) return true;
  try {
    await load(typeof onProgress === "function" ? onProgress : () => {});
    return true;
  } catch (e) {
    console.warn("pose model failed to load", e);
    landmarker = null;
    return false;
  }
}

/**
 * Detect the pose in the current video frame.
 * Returns the number of people found (0 or 1). Results are read with
 * poseLandmarks() and poseMask().
 */
export function poseDetect(video, timestampMs) {
  if (!landmarker || video.readyState < 2) return 0;
  let found = 0;
  // The callback form lets MediaPipe free the mask right after we copy it.
  landmarker.detectForVideo(video, timestampMs, (result) => {
    const first = result.landmarks && result.landmarks[0];
    if (!first) return;
    found = 1;
    for (let i = 0; i < 33; i++) {
      const p = first[i];
      landmarks[i * 3] = p.x;
      landmarks[i * 3 + 1] = p.y;
      landmarks[i * 3 + 2] = p.visibility === undefined ? 1 : p.visibility;
    }
    const m = result.segmentationMasks && result.segmentationMasks[0];
    if (m && wantMask) {
      const f = m.getAsFloat32Array();
      if (mask.length !== f.length) mask = new Uint8Array(f.length);
      for (let i = 0; i < f.length; i++) mask[i] = f[i] * 255;
      maskW = m.width;
      maskH = m.height;
    }
  });
  return found;
}

/** Ask for (or stop asking for) the segmentation mask on later frames. */
export function poseWantMask(on) { wantMask = !!on; }
export function poseLandmarks() { return landmarks; }
export function poseMask() { return mask; }
export function poseMaskWidth() { return maskW; }
export function poseMaskHeight() { return maskH; }

/**
 * Check a still photo before upload. Returns:
 *   -1 model unavailable (skip the check), 0 nobody, 1 one person with the
 *   torso visible, 2 more than one person, 3 one person but torso cut off.
 */
export async function poseCheckImage(image) {
  if (!loading || !fileset || !modelBytes) return -1;
  try {
    if (!imageLandmarker) {
      imageLandmarker = await loading.PoseLandmarker.createFromOptions(fileset, {
        baseOptions: { modelAssetBuffer: modelBytes, delegate: "CPU" },
        runningMode: "IMAGE",
        numPoses: 2,
      });
    }
    const result = imageLandmarker.detect(image);
    const people = result.landmarks ? result.landmarks.length : 0;
    if (people === 0) return 0;
    if (people > 1) return 2;
    const p = result.landmarks[0];
    const visible = (i) => (p[i].visibility === undefined ? 1 : p[i].visibility) > 0.5 && p[i].x >= 0 && p[i].x <= 1 && p[i].y >= 0 && p[i].y <= 1;
    // Shoulders (11, 12) and hips (23, 24).
    return visible(11) && visible(12) && visible(23) && visible(24) ? 1 : 3;
  } catch (e) {
    console.warn("photo pose check failed", e);
    return -1;
  }
}
