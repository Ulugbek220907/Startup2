// Thin wrappers around browser, Telegram and Capacitor APIs.
//
// WHY THIS FILE EXISTS: these are host-environment globals (window.Telegram,
// window.Capacitor, navigator.mediaDevices, navigator.share) with no
// maintained Rust bindings. Everything else in the app is Rust. Each
// function is small, has no app logic, and is imported into Rust through a
// typed `#[wasm_bindgen]` declaration in `src/platform.rs`.

/** Base URL of the API: "" on the website, the server origin in the apps. */
export function apiBase() {
  const base = (window.QD_CONFIG && window.QD_CONFIG.apiBase) || "";
  return base.replace(/\/+$/, "");
}

// ---------------------------------------------------------------- Telegram

function tg() {
  const app = window.Telegram && window.Telegram.WebApp;
  // The SDK object exists in any browser once the script is loaded; it only
  // has initData when really running inside Telegram.
  return app && app.initData ? app : null;
}

/** Signed initData string, or "" when not running inside Telegram. */
export function tgInitData() {
  const app = tg();
  return app ? app.initData : "";
}

export function tgLanguage() {
  const app = tg();
  const user = app && app.initDataUnsafe && app.initDataUnsafe.user;
  return (user && user.language_code) || "";
}

/** Tell Telegram the app is ready and use the full height. */
export function tgReady() {
  const app = tg();
  if (!app) return;
  try {
    app.ready();
    app.expand();
    if (app.setHeaderColor) app.setHeaderColor("#0d0e11");
    if (app.setBackgroundColor) app.setBackgroundColor("#0d0e11");
  } catch (_) { /* older Telegram clients lack some methods */ }
}

// ------------------------------------------------------------------ native

function cap() {
  const c = window.Capacitor;
  return c && c.isNativePlatform && c.isNativePlatform() ? c : null;
}

export function isNative() {
  return !!cap();
}

/** "ios", "android" or "web". */
export function platformName() {
  const c = cap();
  return c ? c.getPlatform() : "web";
}

/**
 * Haptic feedback: Capacitor Haptics in the apps, Telegram's haptics in the
 * Mini App, the Vibration API elsewhere. kind: "light" | "success" | "error".
 */
export function haptic(kind) {
  try {
    const c = cap();
    if (c && c.Plugins && c.Plugins.Haptics) {
      const H = c.Plugins.Haptics;
      if (kind === "light") H.impact({ style: "LIGHT" });
      else H.notification({ type: kind === "error" ? "ERROR" : "SUCCESS" });
      return;
    }
    const app = tg();
    if (app && app.HapticFeedback) {
      if (kind === "light") app.HapticFeedback.impactOccurred("light");
      else app.HapticFeedback.notificationOccurred(kind === "error" ? "error" : "success");
      return;
    }
    if (navigator.vibrate) navigator.vibrate(kind === "error" ? [30, 40, 30] : 12);
  } catch (_) { /* haptics are best-effort */ }
}

function blobToBase64(blob) {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result).split(",")[1] || "");
    r.onerror = () => reject(r.error);
    r.readAsDataURL(blob);
  });
}

/**
 * Share a link and/or an image through the system share sheet.
 * Resolves to "shared", "copied" (link copied to the clipboard instead),
 * "downloaded" (image saved instead) or "failed".
 */
export async function share(title, url, blob, filename) {
  try {
    const c = cap();
    if (c && c.Plugins && c.Plugins.Share) {
      const opts = { title };
      if (url) opts.url = url;
      if (blob && c.Plugins.Filesystem) {
        // The native share sheet needs a file on disk.
        const data = await blobToBase64(blob);
        const file = await c.Plugins.Filesystem.writeFile({ path: filename, data, directory: "CACHE" });
        opts.files = [file.uri];
      }
      await c.Plugins.Share.share(opts);
      return "shared";
    }
    if (navigator.share) {
      const data = { title };
      if (url) data.url = url;
      if (blob) {
        const file = new File([blob], filename, { type: blob.type });
        if (navigator.canShare && navigator.canShare({ files: [file] })) data.files = [file];
      }
      if (data.url || data.files) {
        await navigator.share(data);
        return "shared";
      }
    }
  } catch (e) {
    // The user closing the share sheet is not an error worth reporting.
    if (e && e.name === "AbortError") return "shared";
  }
  try {
    if (blob) {
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(a.href), 10000);
      return "downloaded";
    }
    if (url && navigator.clipboard) {
      await navigator.clipboard.writeText(url);
      return "copied";
    }
  } catch (_) { /* fall through */ }
  return "failed";
}

/** Canvas -> PNG/JPEG Blob (canvas.toBlob is callback-based). */
export function canvasBlob(canvas, type, quality) {
  return new Promise((resolve) => canvas.toBlob((b) => resolve(b), type, quality));
}

// ------------------------------------------------------------------ camera

/**
 * Start the camera into a <video>. facing: "user" | "environment".
 * Resolves to "ok", "denied" (no permission) or "unavailable".
 */
export async function startCamera(video, facing) {
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) return "unavailable";
  stopCamera(video);
  try {
    // In the apps, ask through the native permission dialog first. The
    // WebView then gets the camera without a second, web-style prompt.
    const c = cap();
    if (c && c.Plugins && c.Plugins.Camera) {
      let state = await c.Plugins.Camera.checkPermissions();
      if (state.camera !== "granted") state = await c.Plugins.Camera.requestPermissions({ permissions: ["camera"] });
      if (state.camera !== "granted" && state.camera !== "limited") return "denied";
    }
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: false,
      // 640x480 keeps pose detection fast on low-end phones.
      video: { facingMode: facing, width: { ideal: 640 }, height: { ideal: 480 }, frameRate: { ideal: 30 } },
    });
    video.srcObject = stream;
    // iOS needs these to play inline without going fullscreen.
    video.setAttribute("playsinline", "");
    video.muted = true;
    await video.play();
    return "ok";
  } catch (e) {
    const name = e && e.name;
    return name === "NotAllowedError" || name === "SecurityError" ? "denied" : "unavailable";
  }
}

export function stopCamera(video) {
  const stream = video && video.srcObject;
  if (stream && stream.getTracks) stream.getTracks().forEach((t) => t.stop());
  if (video) video.srcObject = null;
}

// --------------------------------------------------------- service worker

/** Register the offline cache. Skipped inside the native apps, where the
 *  files are already on the device. */
export function registerServiceWorker() {
  if (cap() || !("serviceWorker" in navigator)) return;
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch(() => { /* offline support is optional */ });
  });
}

// ------------------------------------------------------------ native shell

/**
 * One-time setup inside the Android/iOS apps:
 *  - deep links (qiyiqdoppi://d/<slug> and https://<site>/d/<slug>) are
 *    turned into in-app navigation,
 *  - the Android back button walks the app's history instead of closing it,
 *  - the status bar matches the dark theme,
 *  - push notifications are registered when enabled in config.js.
 * Does nothing in a browser.
 */
export function initNative() {
  const c = cap();
  if (!c || !c.Plugins) return;
  const { App, StatusBar, PushNotifications } = c.Plugins;
  const openPath = (path) => {
    history.pushState(null, "", path);
    // The Rust router listens for popstate.
    window.dispatchEvent(new PopStateEvent("popstate"));
  };
  if (App) {
    App.addListener("appUrlOpen", (event) => {
      try {
        const url = new URL(event.url);
        // qiyiqdoppi://d/abc parses as host "d", path "/abc".
        const path = url.protocol === "qiyiqdoppi:" ? "/" + url.host + url.pathname : url.pathname;
        if (/^\/(d|track)\/[A-Za-z0-9-]+$/.test(path)) openPath(path);
      } catch (_) { /* ignore malformed links */ }
    });
    App.addListener("backButton", ({ canGoBack }) => {
      if (canGoBack) history.back();
      else App.exitApp();
    });
  }
  if (StatusBar) {
    StatusBar.setStyle({ style: "DARK" }).catch(() => {});
    if (c.getPlatform() === "android") StatusBar.setBackgroundColor({ color: "#0d0e11" }).catch(() => {});
  }
  // Push groundwork. Off by default: turning it on needs Firebase (Android)
  // and an APNs key (iOS), see docs/MOBILE.md. The token is kept locally
  // until the server-side sender exists (v2).
  if (PushNotifications && window.QD_CONFIG && window.QD_CONFIG.push === true) {
    PushNotifications.addListener("registration", (token) => {
      try { localStorage.setItem("qd.push_token", JSON.stringify(token.value)); } catch (_) {}
    });
    PushNotifications.requestPermissions().then((r) => {
      if (r.receive === "granted") PushNotifications.register();
    }).catch(() => {});
  }
}

/** Remove the boot splash once the app has mounted. */
export function removeBootSplash() {
  const el = document.getElementById("boot");
  if (el) el.remove();
}
