# Secrets and release setup

Secrets live in two places only: **GitHub Actions secrets** (for building
and deploying) and the **`.env` file on the server** (for running). Never
commit a keystore, certificate, key file or `.env`; `.gitignore` already
refuses the usual file types. Add secrets under *Repository -> Settings ->
Secrets and variables -> Actions*.

## Overview

| Name | Used by | What it is |
|---|---|---|
| `VPS_HOST`, `VPS_USER`, `VPS_SSH_KEY` | deploy | Server address, login user, private SSH key |
| `ANDROID_KEYSTORE_BASE64` | android | The release keystore file, base64-encoded |
| `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | android | Keystore credentials |
| `PLAY_SERVICE_ACCOUNT_JSON` (optional) | android | Google service account key for Play uploads |
| `IOS_CERTIFICATE_P12_BASE64`, `IOS_CERTIFICATE_PASSWORD` | ios | Apple Distribution certificate + its export password |
| `IOS_PROFILE_BASE64` | ios | App Store provisioning profile |
| `APPLE_TEAM_ID` | ios | 10-character team id |
| `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_KEY_P8_BASE64` | ios | App Store Connect API key |

Repository **variables** (not secret): `QD_API_BASE` (e.g.
`https://qiyiqdoppi.uz`) and `APP_LINK_HOST` (e.g. `qiyiqdoppi.uz`).

`GITHUB_TOKEN` is provided by GitHub automatically; nothing to create.

## Deploy keys (VPS)

```sh
ssh-keygen -t ed25519 -f deploy_key -C "github-deploy" -N ""
ssh-copy-id -i deploy_key.pub deploy@your-server       # a user in the `docker` group
```

- `VPS_SSH_KEY`: the whole contents of `deploy_key` (the private half)
- `VPS_USER`: `deploy`; `VPS_HOST`: the server's address
- Delete the local `deploy_key` file afterwards.

Create a GitHub *environment* named `production` (Settings -> Environments)
and, if you want a manual approval before each rollout, add yourself as a
required reviewer.

## Android

### 1. Create the upload keystore (once)

```sh
keytool -genkeypair -v -keystore release.keystore -alias qiyiqdoppi \
  -keyalg RSA -keysize 4096 -validity 10000 -storetype PKCS12
```

Choose a strong password. **Back the keystore and password up offline**
(password manager + a USB stick). Without it you cannot publish updates
unless Play App Signing is enabled (step 2), in which case Google can reset
the upload key.

```sh
base64 -w0 release.keystore      # macOS: base64 -i release.keystore
```

- `ANDROID_KEYSTORE_BASE64`: that output
- `ANDROID_KEYSTORE_PASSWORD`: the keystore password
- `ANDROID_KEY_ALIAS`: `qiyiqdoppi`
- `ANDROID_KEY_PASSWORD`: the same password (PKCS12 uses one)

### 2. Play Console app (once)

1. Create a developer account at play.google.com/console (one-time fee;
   identity verification can take days).
2. *Create app*: name QiyiqDoʻppi, default language Uzbek, type App, Free.
3. Package name is `uz.qiyiqdoppi.app`. Changing it later means a new app,
   so decide now (it is set in `mobile/capacitor.config.json` and
   `mobile/android/app/build.gradle`).
4. Accept **Play App Signing** when uploading the first bundle.
5. Fill in the store listing, privacy policy URL (`https://<site>/legal`),
   data safety form (phone number, name, address, photos: collected, not
   shared, deletable) and content rating.
6. The **first** AAB must be uploaded by hand in the Play Console (Internal
   testing -> Create release). The API only works after that.

### 3. Automatic uploads (optional)

1. Google Cloud console: create a project, enable *Google Play Android
   Developer API*, create a service account, add a JSON key.
2. Play Console -> Users and permissions -> invite the service account's
   email with "Release to testing tracks" for this app.
3. `PLAY_SERVICE_ACCOUNT_JSON`: the whole JSON file contents.

Without this secret the workflow still attaches the APK and AAB to the
GitHub release.

### 4. App Links (https links open the app)

After the first Play upload, copy the *App signing key* SHA-256 from
Play Console -> Setup -> App signing, and serve this at
`https://<site>/.well-known/assetlinks.json` (put the file in
`crates/web/public/.well-known/` and add a `copy-dir` line for it in
`index.html`, or serve it from Caddy):

```json
[{ "relation": ["delegate_permission/common.handle_all_urls"],
   "target": { "namespace": "android_app", "package_name": "uz.qiyiqdoppi.app",
               "sha256_cert_fingerprints": ["AA:BB:..."] } }]
```

The custom scheme `qiyiqdoppi://d/<slug>` works without this.

## iOS

Needs an Apple Developer Program membership (yearly fee) and, for steps 2
and 3, a Mac.

### 1. App id and App Store Connect record (once)

1. developer.apple.com -> Certificates, IDs & Profiles -> Identifiers ->
   **+** -> App IDs -> App. Bundle ID (explicit): `uz.qiyiqdoppi.app`.
   Tick *Associated Domains* and *Push Notifications* if you plan to use
   universal links or push.
2. appstoreconnect.apple.com -> Apps -> **+** New App: platform iOS, that
   bundle id, primary language Uzbek (or Russian), SKU `qiyiqdoppi`.
3. `APPLE_TEAM_ID`: Membership details -> Team ID.

### 2. Distribution certificate

1. On a Mac: Keychain Access -> Certificate Assistant -> *Request a
   Certificate from a Certificate Authority* -> save to disk.
2. developer.apple.com -> Certificates -> **+** -> *Apple Distribution* ->
   upload the request -> download the `.cer` -> double-click to install.
3. Keychain Access -> My Certificates -> right-click the "Apple
   Distribution" entry -> Export -> `.p12` with a password.
4. `base64 -i dist.p12 | pbcopy` -> `IOS_CERTIFICATE_P12_BASE64`;
   the export password -> `IOS_CERTIFICATE_PASSWORD`.

The certificate is valid for one year. Put a reminder in your calendar.

### 3. Provisioning profile

1. developer.apple.com -> Profiles -> **+** -> *App Store Connect*
   (distribution) -> app id `uz.qiyiqdoppi.app` -> the certificate above ->
   name it -> download.
2. `base64 -i QiyiqDoppi.mobileprovision | pbcopy` -> `IOS_PROFILE_BASE64`.

It expires with the certificate; regenerate both together.

### 4. App Store Connect API key (for the TestFlight upload)

1. App Store Connect -> Users and Access -> Integrations -> App Store
   Connect API -> **+**, role *App Manager*.
2. Download `AuthKey_XXXXXXXXXX.p8` (possible once only).
3. `ASC_KEY_ID`: the key id; `ASC_ISSUER_ID`: the issuer id shown above the
   list; `ASC_KEY_P8_BASE64`: `base64 -i AuthKey_XXXXXXXXXX.p8`.

### 5. Manual Xcode step for universal links (optional)

Opening `https://<site>/d/...` in the app needs the Associated Domains
entitlement, which must be added in Xcode (Signing & Capabilities ->
+ Capability -> Associated Domains -> `applinks:<site>`), committed, and
the profile regenerated. The custom scheme works without it.

## Release checklist

1. [ ] `main` is green in CI
2. [ ] `CHANGELOG.md` has an entry for the version
3. [ ] If a database migration was added: restore-tested backup exists
4. [ ] `git tag v1.2.3 && git push origin v1.2.3`
5. [ ] **Deploy** workflow finished; `/readyz` is ok; place a test order
6. [ ] **Android** workflow finished; install the APK from the release on a
       real phone and run section 8 of `docs/MANUAL_TESTS.md`
7. [ ] **iOS** workflow finished; the build appears in TestFlight (Apple's
       processing takes 10-30 minutes); install and test
8. [ ] Promote the builds to production in Play Console / App Store Connect
       when satisfied (this step is deliberately manual)

## Status of these pipelines

The workflows and the fastlane lane were written against the tools'
documentation but **have not been executed**: this repository has had no CI
run yet, and no keystore, Apple account or VPS was available while building
it. Expect to fix small things on the first run of each (most likely:
action versions, the Xcode scheme name, signing details). Run the PR
variants first; they need no secrets.
