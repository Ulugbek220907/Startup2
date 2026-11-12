# ADR 0010: Admin sessions, CSRF and rate limiting

**Status:** accepted

- **Customers have no accounts.** An order is read back with its code plus
  the phone number; both must match and wrong guesses are rate limited.
  A design is edited with a secret token kept in the creator's browser.
- **Admin passwords:** Argon2id (PHC string with salt). Login verifies
  against a dummy hash for unknown users so timing does not reveal names.
- **Sessions:** 32 random bytes in an `HttpOnly; SameSite=Strict` cookie
  scoped to `/api/admin`; only the SHA-256 of the token is stored. `Secure`
  is set whenever the site is served over HTTPS.
- **CSRF:** every state-changing admin request must carry the session's
  CSRF token in `X-CSRF-Token`. Enforced in the `AdminSession` extractor,
  so a new admin route cannot forget it.
- **CORS:** only origins in `CORS_ALLOWED_ORIGINS` (the app shells), never
  with credentials, so the admin cookie cannot be used cross-origin.
- **Rate limits:** fixed windows stored in PostgreSQL (per IP, per phone,
  per device). Chosen over in-memory counters so limits survive restarts.
- **CSP:** no inline scripts except the bundler's boot script, whose hash
  is computed at startup; `frame-ancestors` allows Telegram for the
  customer app (Mini Apps are framed) and nothing for admin and API.
- **Audit log:** logins, failed logins, status changes, moderation
  decisions, takedowns, price edits and privacy deletions.

Not included in v1: two-factor login for admin, and automatic account
lockout beyond rate limiting. Use a long unique admin password.
