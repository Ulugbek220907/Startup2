# ADR 0005: Object storage behind a five-method trait

**Status:** accepted

## Decision
`storage::ObjectStore` has `put`, `get`, `delete`, `list`, `ping`.
Implementations: S3-compatible (via the `object_store` crate; MinIO in
development, Cloudflare R2 or similar in production), a local directory,
and in-memory for tests. The server always proxies files; it never hands
out storage URLs, so buckets stay private and access rules live in one
place.

## Consequences
- Switching providers is configuration only.
- Print files (a few MB) pass through the server on download. Acceptable
  for one admin; presigned URLs can be added later if needed.
- Keys are validated (no `..`, no absolute paths) even though only the
  server generates them.
