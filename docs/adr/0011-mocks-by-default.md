# ADR 0011: Every integration is a trait with a working mock

**Status:** accepted

`docker compose up` must work with no API keys. Each third-party
integration is a trait in `crates/providers`, and configuration selects the
implementation:

| Integration | Default | Real implementation in v1 |
|---|---|---|
| Photo try-on | `MockTryOn` (composites the garment on the photo, marked with a striped band) | FASHN (`TRYON_PROVIDER=fashn`) |
| Background removal | `KeyColorRemover` (real local algorithm for flat backgrounds) | none yet |
| Image moderation | `MockModerator` (allow all, flag all, or flag by hash) | none yet |
| SMS | `LogSms` | none (v2) |
| Delivery fee | `TableDelivery` (the admin's fee table; this is the real v1 behaviour) | courier API (v2) |
| Messaging | `LogMessenger` | Telegram bot, when a token is set |

Stated plainly: **image moderation has no real provider in v1.** Text is
checked against the blocklist; images are only checked if you set the mock
to `flag_all` (every design with an image then waits for manual review,
which is a reasonable policy for a small workshop). Background removal
works for logos on plain backgrounds, not for cutting people out of photos.
