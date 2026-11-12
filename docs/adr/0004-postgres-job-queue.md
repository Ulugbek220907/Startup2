# ADR 0004: Background jobs in PostgreSQL

**Status:** accepted

## Decision
A `jobs` table is the queue. Workers claim a row with
`UPDATE ... WHERE id = (SELECT ... FOR UPDATE SKIP LOCKED LIMIT 1)`, which
gives each job to exactly one worker without blocking the others. A claim
sets a 10-minute lease; a crashed worker's job becomes claimable again when
the lease expires. Failures retry with exponential backoff (30 s, 2 min,
8 min, ...) up to `max_attempts`.

Jobs are inserted in the **same transaction** as the data they belong to
(an order and its render/notify jobs commit together or not at all).

## Consequences
- No broker to run, back up or monitor.
- Delivery is at-least-once, so handlers are idempotent (file uploads
  overwrite, status updates are conditional).
- Throughput is bounded by Postgres, far above what a print workshop needs.
- Tests in `crates/server/tests/jobs.rs` cover exclusivity under
  concurrency, retry/backoff and lease recovery.
