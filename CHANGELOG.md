# Changelog

## 0.1.0 — unreleased

First complete version of the v1 scope.

### Added
- **Catalog**: T-shirt, hoodie, sweatshirt; colours, sizes, front/back
  print areas in centimetres; seeded prices and the 14 regions.
- **Design editor**: text (6 fonts, colour, outline, curve, alignment),
  uploaded images, freehand drawing, shapes, stickers, templates;
  move/scale/rotate/flip, snapping, undo/redo, local draft, share link;
  upload checks (real file type, size, EXIF stripped, DPI warning);
  background removal for flat backgrounds; foil finishes shown as "coming
  soon".
- **Renderer**: garment mockups, 300-DPI transparent print files at exact
  physical size, A4 production sheet with QR code; golden-image tests.
- **Live camera try-on**: on-device pose model, mesh warp with sleeves,
  smoothing, arm occlusion, tips, snapshot/share, colour switching,
  fallbacks.
- **Photo try-on**: consent screen, pre-checks, queue, mock provider and
  FASHN client, per-device and global limits, daily cost cap, caching,
  24-hour deletion, "delete now"; evaluation harness.
- **Orders**: server-side pricing (pure function with property tests),
  guest checkout by phone, cash on delivery, pickup or delivery, tracking
  by order number + phone.
- **Admin**: order list with filters, print file and sheet downloads,
  status changes with history, moderation queue, reports and takedowns,
  price / print-area / delivery-fee editor, blocklist, audit log.
- **Telegram**: bot (phone linking, /orders, /status), admin alerts,
  customer status messages, Mini App sign-in with signature verification.
- **Languages**: Uzbek (Latin), Russian, English; auto-detected.
- **Security and operations**: Argon2id, session cookies, CSRF, strict CSP
  and CORS, rate limiting, request ids, health/readiness/metrics, job queue
  in PostgreSQL, retention clean-up.
- **Packaging**: Dockerfile, development and production compose files,
  Caddy config, Capacitor Android and iOS projects, four GitHub Actions
  workflows, runbooks.

### Design changes from the supplied Stitch screens
- Text sizes raised: 17 px body, nothing under 14 px, 48 px minimum tap
  targets; labels that overflowed their pills were removed or shortened.
- Removed promotional wording ("Premium", "Underground", "AR 60 FPS",
  "Live Sync", "Studio Light", "3D", "24 soat", "Xavfsiz tranzaksiya").
- Removed things v1 does not have: Payme/Click/Uzum, SMS confirmation, map
  picker, courier name and plate, gold-foil pricing, the zip-up jacket,
  side poses.
- One language per screen (no "Oldi (Front)" doubles).
- Bottom navigation is Catalog, Design, Try on, Order, Track.
- The font is Montserrat instead of Plus Jakarta Sans: the latter has no
  Cyrillic in its open-source release.

### Known gaps (details in README "Status" and the ADRs)
- Live try-on has not been run on a physical phone; the 20 fps target is
  untested, and first model load on 4G is about 8-15 s (target was 5 s).
- No real call to FASHN, Telegram, Google Play or Apple was made.
- Android and iOS projects were never compiled; CI workflows never ran.
- No real image-moderation provider.
- Text layout has no kerning.
- Translations need a native-speaker review.

### Fixed during development
- Async tasks that finished after their screen was closed read disposed
  state and crashed the app; they now use non-panicking accessors, and the
  browser test fails on any console error.
