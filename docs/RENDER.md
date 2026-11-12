# Hosting on Render

The repository contains a Render Blueprint (`render.yaml`) that creates:

- one **web service** built from the `Dockerfile`: API, web app, admin
  page, background workers and (if you add a token) the Telegram bot, all
  in one process;
- one **PostgreSQL 16** database;
- one **persistent disk** (5 GB at `/data`) for uploaded artwork, print
  files and try-on photos.

Render facts below were read from Render's documentation on 2026-11-12.
Prices change; check render.com/pricing before you start. With the plans
in the blueprint you pay for three things: the web instance (1 CPU / 2 GB),
the smallest paid Postgres, and the disk.

> **Not tested on Render.** The blueprint was written from the docs and the
> image was built and run locally the way Render runs it (port from `PORT`,
> root user, storage on a mounted folder). It has never been applied to a
> Render account. The uncertain points are listed under
> ["What to check after the first deploy"](#what-to-check-after-the-first-deploy).

## Deploy

1. Push this branch to GitHub (merge it into your default branch, or pick
   the branch in step 3).
2. In the Render dashboard: **New → Blueprint**.
3. Connect the GitHub repository and choose the branch. Render reads
   `render.yaml` and shows the service, database and disk it will create.
4. Click **Apply**.

The first build compiles the whole project on Render's build machine
(2 CPUs). Expect it to be slow: possibly 30-60 minutes. Later builds reuse
cached layers and are faster unless dependencies change. Builds are
cancelled after 120 minutes, and they count against your workspace's
monthly pipeline minutes.

When the deploy is live:

- App: `https://<service-name>.onrender.com`
- Admin: `https://<service-name>.onrender.com/admin/`
  - user: `admin`
  - password: the generated value of `ADMIN_PASSWORD` under the service's
    **Environment** tab
- Health: `/healthz` (process up) and `/readyz` (database and storage)

Database migrations and the catalog seed run automatically at start.
**Set your real prices in Admin → Prices**; the seeded ones are examples.

From then on every push to the branch redeploys. Because the service has a
disk, Render stops the old instance before starting the new one, so each
deploy means a few seconds of downtime.

## What to check after the first deploy

1. **`/readyz` says `storage: ok`.** If it says `FAIL`, the app cannot
   write to the disk.
2. **Uploads survive a redeploy.** Upload an image in the editor, place an
   order, trigger a manual deploy, then open the order in admin: the
   mockups must still load. If they vanish, the disk is not mounted where
   the app writes (`STORAGE_FS_DIR` must be inside the disk's `mountPath`).
3. **The container user.** The blueprint sets `RUN_AS=root`, which the
   Docker build receives as a build argument, so the app runs as root and
   can write to a root-owned disk. Whether Render's disk is writable for a
   non-root user was not confirmed. To try the safer setup, delete the
   `RUN_AS` variable, redeploy and repeat check 1; put it back if storage
   fails.
4. **Client IP for rate limiting.** The blueprint trusts the
   `cf-connecting-ip` header (Render's edge runs on Cloudflare), falling
   back to the last `X-Forwarded-For` entry. Render's docs do not describe
   these headers, so verify: sign in to admin, then connect to the
   database (dashboard → database → **Connect** → external `psql` command)
   and run

   ```sql
   SELECT ip, action, created_at FROM audit_log ORDER BY created_at DESC LIMIT 5;
   ```

   `ip` should be your own public address (compare with any "what is my
   IP" site). If it is the same Render or Cloudflare address for everyone,
   rate limits would apply to all customers together; tell me what you see
   in the column and the header setting can be corrected.
5. **Memory.** Place an order and watch the service's **Metrics** tab
   while the print file renders (a few seconds after ordering). If memory
   stays well under the plan's limit you may be able to move down a plan;
   if the instance restarts, move up.

## Your own domain

1. Service → **Settings → Custom Domains** → add the domain and create the
   DNS records Render shows. Render issues the TLS certificate.
2. **Environment** → add `PUBLIC_BASE_URL=https://your-domain` (used in QR
   codes, admin alerts and share links) and save; the service redeploys.

## Turning on the real integrations

Add these under **Environment** (see `.env.example` for all of them):

| Feature | Variables |
|---|---|
| Telegram bot and alerts | `TELEGRAM_BOT_TOKEN`, `TELEGRAM_BOT_USERNAME`, `TELEGRAM_ADMIN_CHAT_ID` (steps in `docs/DEPLOY.md`, "Turn on the Telegram bot") |
| Real photo try-on | `TRYON_PROVIDER=fashn`, `FASHN_API_KEY`, `TRYON_COST_PER_IMAGE_SUM`, `TRYON_DAILY_COST_CAP_SUM` (read ADR 0008 and `docs/PRIVACY.md` first) |
| Files in S3 / Cloudflare R2 instead of the disk | `STORAGE_BACKEND=s3`, `S3_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY`, `S3_SECRET_KEY`, `S3_PATH_STYLE` |

Mark tokens and keys as secret values in the dashboard. Never put them in
`render.yaml`: that file is in git.

The bot uses long polling, which needs the service to stay running. That
is the case on every paid plan.

## Backups

- **Database:** see the database's **Recovery** tab for what your plan
  includes. Independently of that, you can take your own dumps with the
  external connection string:
  `pg_dump -Fc "<external database URL>" > qiyiq-$(date +%F).dump`
  and restore with `pg_restore --no-owner -d "<URL>" file.dump`.
- **Disk:** Render snapshots it automatically every 24 hours and keeps
  snapshots for seven days (restore from the service's **Disks** page).
  Print files and mockups can also be regenerated from the database with
  "render again" on an order, as long as the uploaded artwork survives.

`docs/DEPLOY.md` sections 4-7 (storage layout, monitoring queries, routine
tasks, launch checklist) apply on Render too; replace the
`docker compose exec db psql ...` commands with Render's `psql` connect
command. `/metrics` is reachable publicly on Render (there is no Caddy in
front to block it). It only exposes counters, no personal data, but if you
prefer it closed, say so and it can be put behind a token.

## Trying it on the free plan

Possible for a look, not for real use:

- Free web services cannot have a disk, so files are lost on every restart
  and deploy; they sleep after 15 minutes without traffic (the next visitor
  waits about a minute, and the bot and background jobs stop meanwhile);
  and they have 512 MB of memory and a tenth of a CPU, which may not be
  enough to render a print file.
- A free database is deleted 30 days after creation.

To try anyway, edit `render.yaml` before applying it: set both `plan`
values to `free`, delete the `disk:` block and the `RUN_AS` variable, and
set `STORAGE_FS_DIR` to `/tmp/storage`.

## Region

The blueprint uses `frankfurt`, the closest Render region to Uzbekistan
(the others are in the US and Singapore). The region cannot be changed
later. Data therefore sits in Germany; whether customer data may be stored
outside Uzbekistan is one of the questions for your lawyer in
`docs/PRIVACY.md`. If it must stay in the country, use a local VPS with
`docs/DEPLOY.md` instead.

## If the build is too slow or fails on Render

Render can also run a prebuilt image (`runtime: image`) instead of
building. The image would be built by GitHub Actions and pushed to the
GitHub Container Registry, and Render told to pull it through a deploy
hook. That path is **not set up** in this repository; ask for it if the
Docker build on Render turns out to be a problem.

## Mobile apps

Build them against the Render address (or your domain):

```sh
QD_API_BASE=https://<service-name>.onrender.com scripts/build-mobile-web.sh
```

and set the same value as the `QD_API_BASE` repository variable for the
Android and iOS workflows.

## GitHub Actions

`deploy.yml` is for the VPS setup and is skipped unless the repository
variable `DEPLOY_TARGET` is `vps`. On Render you do not need it. If you
want Render to deploy only after CI passes, change `autoDeployTrigger` in
`render.yaml` from `commit` to `checksPass`.
