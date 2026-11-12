# Mobile apps (Android and iOS)

The apps are the web app packaged with Capacitor 8 (see ADR 0007). The
native projects are committed in `mobile/android` and `mobile/ios`.

## Build locally

Needs: everything for the web build, Node 22, and Android Studio (JDK 21)
and/or Xcode 26.

```sh
QD_API_BASE=https://your-site.example scripts/build-mobile-web.sh
cd mobile
npx cap open android      # then Run in Android Studio
npx cap open ios          # then Run in Xcode
```

`QD_API_BASE` must be an **https** address the phone can reach, and the
server must allow the app origins:

```
CORS_ALLOWED_ORIGINS=https://localhost,capacitor://localhost
```

(`https://localhost` is the Android WebView origin, `capacitor://localhost`
the iOS one.) To test against a laptop, expose it with an HTTPS tunnel.

## What is native

| Feature | How |
|---|---|
| Camera permission | Native dialog through the Camera plugin before the WebView asks; `CAMERA` in the Android manifest; `NSCameraUsageDescription` on iOS |
| Photo picker | System picker from the file input; `NSPhotoLibraryUsageDescription` on iOS |
| Haptics | Haptics plugin on taps, success and errors |
| Share | Native share sheet for design links and try-on snapshots |
| Back button (Android) | Walks the in-app history, exits from the catalog |
| Deep links | `qiyiqdoppi://d/<slug>`; `https://<site>/d/<slug>` after the store setup in `docs/SECRETS.md` |
| Offline shell | All app files are on the device; without network the app opens and offers "retry" |
| Status bar, splash, icons | Dark theme; generated from `mobile/assets` with `npm run assets` |
| Push notifications | Groundwork only, see below |

## Push notifications (groundwork)

The plugin is installed and `initNative()` in `crates/web/shims/platform.js`
registers for push when the build sets `QD_PUSH=true`. It is **off by
default** and nothing sends pushes yet. To finish it in v2:

1. Android: create a Firebase project, add the app, put
   `google-services.json` in `mobile/android/app/` (the Gradle file already
   applies the Google services plugin when that file exists).
2. iOS: enable the Push Notifications capability in Xcode and create an
   APNs key.
3. Server: store the device token against the customer (the app keeps it in
   local storage for now) and send through FCM/APNs from the
   `notify_customer` job.

Do not set `QD_PUSH=true` on Android before step 1: registering without
Firebase configuration crashes the app.

## Changing the app id, name or icon

- Id: `mobile/capacitor.config.json`, `mobile/android/app/build.gradle`
  (`applicationId`, `namespace`), the Android package folders, the iOS
  bundle identifier in Xcode, and the fastlane files. Decide before the
  first store upload.
- Name: `capacitor.config.json`, `android/app/src/main/res/values/strings.xml`,
  `CFBundleDisplayName` in `ios/App/App/Info.plist`.
- Icon: edit `crates/web/public/icons/icon.svg`, run
  `cargo run -p renderer --example icons`, then `cd mobile && npm run assets`.

## Not verified here

The Android and iOS projects were generated and configured, but **never
compiled**: no Android SDK or Xcode was available. Camera + WASM + the pose
model inside the Android WebView and WKWebView are expected to work (both
are Chromium/WebKit with `getUserMedia` in a secure context) but this must
be confirmed on real devices with `docs/MANUAL_TESTS.md` sections 3 and 8.
iOS requires 14.5 or newer for the camera in a WebView; the project targets
iOS 15.
