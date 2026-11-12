# QiyiqDoʻppi

Custom apparel for Uzbekistan, fully self-serve: pick a T-shirt, hoodie or
sweatshirt, design it with your own text, pictures and drawings, **see it on
yourself** (live camera or a photo), get the price instantly and send the
order. The workshop gets a 300-DPI print file and a production sheet.

One Rust codebase: an axum backend, a Leptos (WebAssembly) web app that is
also a PWA and a Telegram Mini App, and Android/iOS apps built from the
same web build with Capacitor.

- [Quick start](#quick-start)
- [What works, and what has not been verified](#status)
- [Architecture](#architecture)
- [Configuration](#configuration)
- [Development](#development)
- [Testing](#testing)
- [Documentation index](#documentation)

## Quick start

Needs Docker with the compose plugin. No API keys.

```sh
docker compose up --build
```

The first build compiles everything and takes a while (about 15-25
minutes); later builds are much faster. Then:

| | |
|---|---|
| App | http://localhost:8080 (open it at phone width, or on a phone on the same network) |
| Admin | http://localhost:8080/admin/ — user `admin`, password `admin-dev-password` |
| MinIO console | http://localhost:9001 — `qiyiq-dev` / `qiyiq-dev-secret` |

The database is seeded with three garments, prices, the 14 regions and one
demo order. Everything external is mocked: photo try-on returns a
composited preview with a striped band, Telegram messages are written to
the log.

If Docker Hub rate-limits the base images, add
`--build-arg BASE_REGISTRY=public.ecr.aws/docker/library` (with
`docker compose build`).

Try the whole flow:

1. Choose **Xudi**, tap **Matn**, type `Oʻzbekiston`, bend it with the curve slider.
2. **Kiyib koʻrish** → allow the camera (needs https or localhost) → the
   hoodie follows your body. Or use **Rasm orqali** with a photo.
3. **Buyurtma berish** → size, region, phone → send. Note the order number.
4. In the admin page: open the order, download the print PNG and the
   production sheet PDF, change the status.

The camera only works on `https://` or `localhost`. To test on a phone
against your computer, use an HTTPS tunnel or deploy.

## Status

Built and verified here:

- Backend, renderer and business rules: **129 automated tests**, including
  property tests for money and pricing, golden-image tests for mockups,
  print files and the production sheet, and integration tests against real
  PostgreSQL (orders, admin, moderation, privacy, try-on with the mock,
  the job queue).
- A browser end-to-end test on an emulated Pixel 5: design with Uzbek text
  → preview → order → tracking → admin downloads a 3543×4134 px print file
  (30×35 cm at 300 DPI) → status change reaches the customer.
- Live camera try-on, in headless Chromium with a fake camera: the model
  loads, the pose is found and the garment is warped onto the body.
- First-visit payload: about 400 KB gzipped (budget 600 KB, enforced by the
  build script).
- The Docker image (203 MB, non-root user, built-in healthcheck) and the
  compose stack with PostgreSQL and S3 storage (MinIO): the browser test
  passes against the containers. See the caveat in the table below.
- `cargo fmt`, `clippy -D warnings` and `cargo deny` are clean.

**Not verified. Please read before relying on any of it:**

| Item | State |
|---|---|
| Live try-on on real phones | Never run on a physical device. The 20 fps target is untested. Use `docs/MANUAL_TESTS.md`. |
| Model load under 5 s on 4G | **Not met** on a first visit: about 10 MB must download (8-15 s on 4G), then it is cached. See ADR 0009. |
| Real photo try-on (FASHN) | Client written from the vendor docs and tested against a local fake of the API. No real call was made (no key). One endpoint detail is unconfirmed; see ADR 0008. |
| Telegram bot | Compiles; commands and message formatting are unit-tested; initData verification is tested. Never connected to Telegram (no token). |
| Android and iOS builds | Projects generated and configured, never compiled (no Android SDK or Xcode here). |
| GitHub Actions workflows | Written, YAML-valid, **never run**. Action versions could not be checked from the build environment. See `docs/VERSIONS.md`. |
| Render deployment | `render.yaml` written from Render's docs; the image was run locally the way Render runs it. Never applied to a Render account. See `docs/RENDER.md`. |
| Signed store builds from a tag | Depends on the two rows above plus your keystore and Apple account. Expect fixes on first run. |
| `docker compose up --build` exactly as written | The image was built from this Dockerfile and the compose stack (app + PostgreSQL + MinIO) was started and passed the browser test, **but** the build environment's network proxy required a temporary Dockerfile variant that trusts its certificate, and the image was built separately and then started with `--no-build`. The unmodified one-command path has not been run on a normal machine. |
| Image moderation | No real provider. Text blocklist + manual review; see ADR 0011. |
| Uzbek and Russian wording | Written by the developer tooling, not reviewed by a native speaker. Have the translation files read before launch. |
| Legal text | None of the wording is legal advice. See `docs/PRIVACY.md`. |

Garment pictures are flat illustrations drawn in code
(`crates/design/src/garment_art.rs`). Replace them with photos of your real
garments when you have them; print areas are edited in the admin page.

**Seed prices are examples.** Set your own in Admin → Prices before launch.

## Architecture

```
 Phone browser / PWA / Telegram Mini App / Capacitor WebView (Android, iOS)
 ┌──────────────────────────────────────────────────────────────────────┐
 │ Leptos app (Rust → WASM)          print fonts and pose model are lazy │
 │  catalog · editor (Canvas 2D) · try-on · order · tracking            │
 │  live try-on: WebGL mesh warp + One-Euro smoothing (Rust)            │
 │  JS shims: platform.js (camera, Telegram, Capacitor), pose.js         │
 └───────────────────────────────┬──────────────────────────────────────┘
                                 │ HTTPS, JSON
 ┌───────────────────────────────▼──────────────────────────────────────┐
 │ qiyiq-server (one binary: axum + tokio)                               │
 │  API · static files · admin sessions · Telegram bot (long polling)    │
 │  workers: render, try-on, moderation, notifications, retention        │
 │  providers behind traits, mocks by default                            │
 └───────────┬──────────────────────────────┬───────────────────────────┘
             │                              │
      PostgreSQL 16                  S3-compatible storage or local disk
      data + job queue               artwork, mockups, print files, sheets
```

### Crates

| Crate | Purpose |
|---|---|
| `design` | The versioned design document; text → outlines; shapes, stickers, templates; garment artwork; try-on mesh math. Shared by browser and server. |
| `domain` | Money (tiyin), phone numbers, pricing, order states, translations, API types. No I/O. |
| `renderer` | Mockup PNG, 300-DPI print PNG, production sheet PDF. Deterministic. |
| `storage` | Object storage trait: S3, local directory, memory. |
| `providers` | Third-party integrations as traits with mocks; FASHN try-on client. |
| `db` | Compile-time-checked SQL, migrations, the job queue. |
| `server` | HTTP API, auth, workers, bot, the binary. |
| `web` | Customer app and admin page (two WASM bundles), JS shims. |
| `evalkit` | Try-on evaluation harness. |

Other folders: `migrations/` (SQL), `mobile/` (Capacitor projects),
`deploy/` (production compose + Caddy), `e2e/` (browser test), `eval/`
(try-on evaluation), `scripts/`, `docs/`.

### Data model (main tables)

`garments`, `garment_colors`, `garment_sizes`, `print_areas` · `price_tiers`,
`quantity_discounts`, `regions`, `settings` · `customers` · `assets`,
`designs`, `design_reports` · `orders`, `order_items`, `order_status_log`,
`order_files`, `payments` (unused until v2) · `tryon_jobs` · `jobs` ·
`admins`, `sessions`, `audit_log`, `blocklist_terms`, `rate_limits`.

Money columns are `BIGINT` tiyin (1 soʻm = 100 tiyin); the code never uses
floats for money (`crates/domain/src/money.rs`).

### API

Public: `GET /api/catalog` · `POST /api/assets`, `GET /api/assets/{id}/preview`,
`POST /api/assets/{id}/remove-bg` · `POST /api/designs`,
`GET|PUT /api/designs/{slug}`, `POST /api/designs/{slug}/render`,
`GET /api/designs/{slug}/mockup/{side}`, `POST /api/designs/{slug}/report` ·
`GET /api/jobs/{id}` · `POST /api/price/quote` · `POST /api/orders`,
`POST /api/orders/lookup` · `POST /api/me/export`, `POST /api/me/delete` ·
`POST /api/tryon`, `GET|DELETE /api/tryon/{id}`, `GET /api/tryon/{id}/result` ·
`POST /api/telegram/me`

Admin (session cookie + CSRF header): `POST /api/admin/login|logout`,
`GET /api/admin/me` · `GET /api/admin/orders`, `GET /api/admin/orders/{id}`,
`POST .../status`, `POST .../rerender`, `GET .../files/{kind}` ·
`GET /api/admin/moderation`, `GET /api/admin/designs/{id}/preview`,
`POST /api/admin/designs/{id}/moderation` · `GET /api/admin/reports`,
`POST /api/admin/reports/{id}` · `GET|PUT /api/admin/pricing`,
`GET|PUT /api/admin/blocklist` · `GET /api/admin/audit`

Operations: `GET /healthz`, `GET /readyz`, `GET /metrics`

Errors are `{"error":{"code":"...","message":"..."}}`; the app translates
`code` (`error.<code>` in the locale files).

### Background jobs

`render_mockup`, `render_order_files` (mockups + print PNGs + sheet),
`moderate_design`, `tryon_generate`, `notify_admin`, `notify_customer`,
`purge_expired`. See ADR 0004.

### Pricing

```
unit     = garment base + Σ print tier(bounding-box area of each printed side)
subtotal = unit × quantity
discount = subtotal × best quantity discount, rounded down to a whole soʻm
total    = subtotal − discount + delivery fee
```

One pure function (`domain::pricing::quote`) used by the server for the
real price and by the editor for the live estimate. Client-sent prices are
ignored. All four inputs are edited in Admin → Prices.

## Configuration

Every variable is documented in [`.env.example`](.env.example). With only
`DATABASE_URL` set, the server starts with safe defaults: mock try-on,
files on local disk, no bot.

## Development

Needs Rust 1.97 (`rustup` reads `rust-toolchain.toml`), PostgreSQL 16,
`trunk` 0.22.1 and `curl`. Docker for the integration tests.

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --version 0.22.1 --locked
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cargo install sqlx-cli --version 0.9.0 --no-default-features --features rustls,postgres --locked

createdb qiyiq
export DATABASE_URL=postgres://localhost/qiyiq
scripts/build-web.sh                       # builds crates/web/dist (downloads the pose model once)
ADMIN_USERNAME=admin ADMIN_PASSWORD=admin-dev-password LOG_FORMAT=pretty SEED_DEMO=true \
  cargo run -p server                      # http://localhost:8080
```

Front-end work with reload: `cd crates/web && trunk serve` (port 8081,
proxies `/api` to 8080). Note that when the server serves the build itself
it must be restarted after each web build, because its content security
policy pins the hash of the page's boot script.

After changing any SQL: `cargo sqlx prepare --workspace` and commit `.sqlx/`.

Adding a UI string: add the key to all three files in
`crates/domain/locales/`; a test fails if they differ.

## Testing

```sh
cargo test                                   # everything except the browser test (needs Docker)
TEST_DATABASE_URL=postgres://localhost/postgres cargo test   # reuse a local Postgres instead of containers
UPDATE_GOLDEN=1 cargo test -p renderer --test golden          # after an intended rendering change
cargo clippy --all-targets -- -D warnings
cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings
cargo deny check

# Browser end-to-end test against a running server:
cd e2e && npm install && npx playwright install chromium && cd ..
ADMIN_PASS=admin-dev-password node e2e/customer-flow.js

# Try-on evaluation harness:
cargo run -p evalkit -- --self-test
```

| What | Where |
|---|---|
| Money and pricing property tests | `crates/domain/src/money.rs`, `pricing.rs` |
| Phone numbers, order state machine, translations | `crates/domain/src/` |
| Text layout, curved text, fonts, try-on mesh, One-Euro filter | `crates/design/src/` |
| Golden images (mockup, print, sheet) | `crates/renderer/tests/golden.rs` |
| Upload safety (magic bytes, bombs, EXIF stripping) | `crates/server/src/uploads.rs` |
| Orders, admin, security headers, moderation, privacy, Telegram auth | `crates/server/tests/e2e.rs` |
| Photo try-on: consent, limits, cost cap, deletion, expiry | `crates/server/tests/tryon.rs` |
| Job queue | `crates/server/tests/jobs.rs` |
| FASHN client against a fake API | `crates/providers/src/tryon.rs` |
| Customer journey in a browser | `e2e/customer-flow.js` |
| Real devices | `docs/MANUAL_TESTS.md` |

## Documentation

| | |
|---|---|
| [docs/RENDER.md](docs/RENDER.md) | Hosting on Render with the included blueprint (`render.yaml`) |
| [docs/DEPLOY.md](docs/DEPLOY.md) | Deploying to a VPS, backups and restore, storage lifecycle, monitoring, routine tasks |
| [docs/SECRETS.md](docs/SECRETS.md) | Every secret, Android keystore, Play Console, Apple setup, release checklist |
| [docs/MOBILE.md](docs/MOBILE.md) | Building and configuring the Android and iOS apps |
| [docs/PRIVACY.md](docs/PRIVACY.md) | What personal data is stored and for how long; try-on provider data handling |
| [docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md) | Device checklist, especially for the camera |
| [docs/VERSIONS.md](docs/VERSIONS.md) | Versions in use and how each was verified |
| [docs/adr/](docs/adr/) | Decision records (architecture, try-on provider comparison, Capacitor vs Tauri, ...) |
| [eval/README.md](eval/README.md) | Evaluating a photo try-on provider |
| [crates/design/fonts/README.md](crates/design/fonts/README.md) | Bundled fonts and their licences |

## Deferred to v2

Online payments (Click/Payme; the `payments` table and the tiyin/soʻm
conversions are ready), SMS, analytics, the foil finish pipeline (modelled
in the schema, disabled in the UI and rejected by the server), inventory,
push notification sending, two-factor admin login, a real image-moderation
provider.

## Licence

The application code is proprietary (`UNLICENSED`). Bundled fonts are SIL
OFL 1.1; MediaPipe and its pose model are Apache-2.0 and are downloaded at
build time, not stored in this repository.
