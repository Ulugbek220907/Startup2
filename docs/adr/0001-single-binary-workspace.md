# ADR 0001: One Cargo workspace, one backend binary

**Status:** accepted

## Context
The product needs an API, background rendering, a Telegram bot and static
file hosting, operated by one person on one VPS.

## Decision
A Cargo workspace of small crates (`design`, `domain`, `renderer`,
`storage`, `providers`, `db`, `server`, `web`, `evalkit`) producing **one**
server binary. The binary serves the API and the built web app, runs the
job workers as tokio tasks and runs the bot with long polling. The only
other processes are PostgreSQL and (optionally) an S3-compatible store.

## Consequences
- Deploying is "pull one image, restart one container".
- Crate boundaries keep pure logic (`design`, `domain`) free of I/O, so it
  compiles to WASM and is easy to test.
- Rendering shares CPU with request handling. A semaphore limits renders to
  one at a time; if that ever becomes a bottleneck, the same binary can be
  started a second time as a worker-only process (the queue already
  supports several consumers).
