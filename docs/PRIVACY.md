# Personal data: what is stored, where, and for how long

This describes what the software does. It is not legal advice.

> **Before launch, ask a lawyer in Uzbekistan to confirm the current
> requirements of the personal-data law for your case**, in particular:
> where personal data of Uzbek citizens must be stored and processed,
> whether your database must be registered, what consent wording is
> required, and whether sending customers' photos to a foreign service is
> allowed and under what conditions. The points below are things to raise
> with them, not conclusions.

## What is collected

| Data | Why | Where | Kept until |
|---|---|---|---|
| Phone number, name (optional), delivery address, notes | To make and deliver the order and confirm it by phone | PostgreSQL (`orders`, `customers`) | The customer deletes it, or you do |
| Design (text, uploaded artwork) | To print it | PostgreSQL + object storage | Ordered designs: with the order. Never-ordered drafts and unused uploads: `DRAFT_TTL_DAYS` (90) |
| Telegram user id and chat id | To show "my orders" and send status messages, only if the customer uses the bot | PostgreSQL (`customers`) | Same as the phone number |
| Try-on photo and generated result | To show the preview | Object storage (`tryon/`) | `TRYON_TTL_HOURS` (24), or immediately on "delete now" |
| IP address | Rate limiting; audit log for admin actions | PostgreSQL (`rate_limits` keys, `audit_log`) | Rate-limit rows: 2 days. Audit log: until you prune it |
| Admin username, password hash, sessions | Admin sign-in | PostgreSQL | Sessions: `SESSION_TTL_HOURS` |

Not collected: payment data (v1 is cash on delivery), location, contacts,
analytics, advertising identifiers. No third-party scripts are loaded
except Telegram's own script inside the Mini App.

Uploaded images are re-encoded on arrival, which removes EXIF metadata
(camera model, GPS position). Logs contain masked phone numbers only and
never message bodies or addresses.

## Live camera try-on

Runs entirely in the browser. Video frames are not uploaded, stored or
logged. The pose model and its runtime are served from your own domain.

## Photo try-on and the external provider

With `TRYON_PROVIDER=mock` (default) photos stay on your server.

With `TRYON_PROVIDER=fashn` each photo is sent to FASHN (api.fashn.ai, a
company outside Uzbekistan) to generate the image. From their
documentation as read on 2026-11-11:

- Inputs sent inline (which is what this app does) are deleted when the
  request finishes, with a one-day backstop.
- Results requested as base64 (which is what this app does) can be read for
  60 minutes; results on their CDN are deleted after three days.
- Request records (ids, parameters) are not deleted automatically; deletion
  can be requested by email.
- Customer content is not used to train models unless you opt in.

Read their current terms yourself before enabling it, and tell customers in
your own privacy text that photos are processed by an external service.
The consent screen shown before upload says the photo is sent to a server
and deleted after the configured time; adjust the wording
(`tryon.consent_*` in `crates/domain/locales/`) to match your final setup.

The app asks the customer to confirm that the photo shows themselves or an
adult who agreed. It cannot verify age or consent.

## Customer rights in the app

On the "Privacy and rules" screen, after entering their phone and one of
their order numbers, a customer can:

- **download their data** as JSON (`POST /api/me/export`);
- **delete their data** (`POST /api/me/delete`): phone, name, address and
  notes are removed from their orders and the customer record is deleted.
  The order's amounts and design are kept as a business record. Deletion is
  refused while an order is still in progress.

To handle a request made by phone or in person, run the same deletion from
`psql` (see `docs/DEPLOY.md`, "Deleting a customer's data by hand").

## Artwork rights, reports and takedowns

Customers confirm at upload and at order time that they may use the
artwork. Anyone can report a shared design; a rights holder can file a
takedown claim with contact details. Reports flag the design for review;
"take design down" in admin removes it from share links and blocks it from
production. Every decision is in the audit log.

## Hosting inside Uzbekistan

Nothing in the stack requires a foreign service: the server, PostgreSQL and
file storage (`STORAGE_BACKEND=fs`, or MinIO on your own machine) can all
run on a VPS located in Uzbekistan, and with the mock or a self-hosted
try-on model no personal data leaves it. Choosing Cloudflare R2, FASHN or a
foreign VPS moves data abroad; whether that is acceptable is the question
for your lawyer.
