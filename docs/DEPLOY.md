# Deploy and operations runbook

One VPS runs everything: Caddy (HTTPS) in front of the app, with PostgreSQL
beside it. 2 vCPU / 4 GB RAM / 40 GB disk is a comfortable start (a
300-DPI render peaks around 150 MB of memory).

## 1. First deployment

On the server (Ubuntu 24.04 assumed):

```sh
# Docker Engine + compose plugin: https://docs.docker.com/engine/install/ubuntu/
sudo mkdir -p /opt/qiyiqdoppi && sudo chown "$USER" /opt/qiyiqdoppi
```

Point your domain's A record (and `www`) at the server and open ports 80
and 443. Then create `/opt/qiyiqdoppi/.env` from `.env.example`. The
minimum for production:

```sh
SITE_DOMAIN=qiyiqdoppi.uz
PUBLIC_BASE_URL=https://qiyiqdoppi.uz
POSTGRES_PASSWORD=<long random string>
ADMIN_USERNAME=<your admin name>
ADMIN_PASSWORD=<long unique password, 10+ characters>
STORAGE_BACKEND=fs            # files on the server's disk (volume app-data)
CORS_ALLOWED_ORIGINS=https://localhost,capacitor://localhost
# Optional: TELEGRAM_BOT_TOKEN, TELEGRAM_BOT_USERNAME, TELEGRAM_ADMIN_CHAT_ID
# Optional: TRYON_PROVIDER=fashn and FASHN_API_KEY
```

```sh
chmod 600 /opt/qiyiqdoppi/.env
```

`DATABASE_URL` and `TRUST_PROXY` are set by the compose file. Generate
secrets with `openssl rand -base64 32`.

Deploy by pushing a version tag (`git tag v1.0.0 && git push --tags`); the
**Deploy** workflow builds the image, copies `docker-compose.prod.yml` and
`Caddyfile` to `/opt/qiyiqdoppi`, and restarts the stack. It needs the
`VPS_HOST`, `VPS_USER` and `VPS_SSH_KEY` secrets (`docs/SECRETS.md`).

To deploy by hand instead:

```sh
docker build -t qiyiqdoppi:1.0.0 .          # on a build machine, then push or `docker save | ssh ... docker load`
cd /opt/qiyiqdoppi
echo "APP_IMAGE=qiyiqdoppi:1.0.0" >> .env
docker compose -f docker-compose.prod.yml --env-file .env up -d
```

Verify:

```sh
curl -fsS https://qiyiqdoppi.uz/healthz      # ok
curl -fsS https://qiyiqdoppi.uz/readyz       # database: ok / storage: ok
docker compose -f docker-compose.prod.yml logs --tail 50 app
```

Then open `/admin/`, sign in, and place a test order from a phone.
Migrations run automatically at start.

## 2. Updating and rolling back

Update: push a new tag. Roll back: set `APP_IMAGE` in `.env` to the
previous tag and run `docker compose ... up -d`. Migrations only add
things in v1, so the previous version keeps working against a newer
schema; if a future migration is not backward compatible it will say so in
`CHANGELOG.md`, and rolling back then means restoring a backup.

## 3. Backups

What must be backed up: **the PostgreSQL database** and **the file
storage** (uploaded artwork, print files). The `.env` file too, kept
somewhere safe and separate.

### Database

Nightly dump via cron on the host (`crontab -e`):

```cron
15 3 * * * cd /opt/qiyiqdoppi && docker compose -f docker-compose.prod.yml exec -T db pg_dump -U qiyiq -Fc qiyiq > /var/backups/qiyiq/db-$(date +\%F).dump && find /var/backups/qiyiq -name 'db-*.dump' -mtime +14 -delete
```

Create `/var/backups/qiyiq` first, and copy the dumps off the server (for
example `rclone copy /var/backups/qiyiq remote:qiyiq-backups`). A backup
that lives only on the same disk is not a backup. The dumps contain
customer phone numbers and addresses: encrypt them or restrict the bucket.

Restore (this **replaces** the current database; stop the app first):

```sh
cd /opt/qiyiqdoppi
docker compose -f docker-compose.prod.yml stop app
docker compose -f docker-compose.prod.yml exec -T db dropdb -U qiyiq qiyiq
docker compose -f docker-compose.prod.yml exec -T db createdb -U qiyiq qiyiq
docker compose -f docker-compose.prod.yml exec -T db pg_restore -U qiyiq -d qiyiq --no-owner < /var/backups/qiyiq/db-2026-11-11.dump
docker compose -f docker-compose.prod.yml start app
```

**Test a restore once before launch**, on a scratch machine, so you know it
works.

### Files

- `STORAGE_BACKEND=fs`: files live in the Docker volume `app-data`.
  ```sh
  docker run --rm -v qiyiqdoppi_app-data:/data -v /var/backups/qiyiq:/backup alpine tar czf /backup/files-$(date +%F).tgz -C /data .
  ```
- `STORAGE_BACKEND=s3`: use the provider's versioning or replication, or
  `rclone sync` to a second bucket.

Print files and mockups can be regenerated from the database ("render
again" on the order), as long as the uploaded artwork (`assets/`) survives.
So `assets/` is the part that truly needs backing up.

## 4. Storage layout and lifecycle rules

| Prefix | Contents | Deleted by |
|---|---|---|
| `assets/<id>/` | Uploaded artwork (original + preview) | The app, when no design or order uses it for `DRAFT_TTL_DAYS` |
| `mockups/<design id>/` | Mockups of saved designs | Never by the app; safe to delete (regenerated on demand) |
| `orders/<order id>/` | Mockups, 300-DPI print files, production sheet | Never by the app |
| `tryon/<id>/` | Try-on photo, garment image, result | The app, after `TRYON_TTL_HOURS` or on "delete now" |

The app's hourly clean-up job enforces the retention above. On S3/R2, add
bucket lifecycle rules as a second line of defence:

- prefix `tryon/`: expire objects after **2 days** (comfortably above the
  24-hour TTL, so it only ever removes what the app should already have
  deleted);
- prefix `mockups/`: expire after 90 days (optional, saves space);
- no rule on `assets/` or `orders/`.

Keep the bucket private. The app never needs public objects.

## 5. Monitoring

- `/healthz`: process is up. `/readyz`: database and storage reachable.
  Point an uptime monitor at `/readyz`.
- `/metrics` (Prometheus text) is blocked by Caddy from outside. Read it on
  the server: `docker compose -f docker-compose.prod.yml exec app qiyiq-server healthcheck`
  for a quick check, or `docker compose ... exec caddy wget -qO- http://app:8080/metrics`.
  Worth watching: `qiyiq_jobs{status="failed"}`, `qiyiq_http_responses_total{class="5xx"}`.
- Logs are JSON lines on stdout: `docker compose ... logs -f app`. Each
  request has an `x-request-id` (also returned to the client) to find it.
- Failed background jobs: `SELECT id, kind, last_error, finished_at FROM jobs WHERE status = 'failed' ORDER BY finished_at DESC LIMIT 20;`

## 6. Routine tasks

**Reset the admin password:** change `ADMIN_PASSWORD` in `.env`, restart
the app. (The account is re-created from the env values at every start.)

**psql shell:**
```sh
docker compose -f docker-compose.prod.yml exec db psql -U qiyiq qiyiq
```

**Deleting a customer's data by hand** (request made by phone):
```sql
BEGIN;
UPDATE orders SET phone = NULL, customer_name = NULL, address = NULL, notes = NULL, customer_id = NULL
 WHERE phone = '+998901234567' AND status IN ('delivered', 'cancelled');
DELETE FROM customers WHERE phone = '+998901234567'
   AND NOT EXISTS (SELECT 1 FROM orders WHERE phone = '+998901234567');
COMMIT;
```

**Prune the audit log** (keep one year):
```sql
DELETE FROM audit_log WHERE created_at < now() - interval '1 year';
```

**Turn on the Telegram bot:** create it with @BotFather, set
`TELEGRAM_BOT_TOKEN` and `TELEGRAM_BOT_USERNAME`; send `/start` to the bot
from your own account and read your chat id from
`SELECT telegram_chat_id FROM customers ORDER BY created_at DESC LIMIT 1;`,
set it as `TELEGRAM_ADMIN_CHAT_ID`; restart. To use the Mini App, set the
bot's menu button URL to your site in @BotFather.

**Turn on real photo try-on:** read ADR 0008 and `docs/PRIVACY.md`, run the
evaluation harness with your key, then set `TRYON_PROVIDER=fashn`,
`FASHN_API_KEY`, `TRYON_COST_PER_IMAGE_SUM` and the caps; restart.

## 7. Security checklist before launch

- [ ] `.env` is `chmod 600`, not in git, and backed up separately
- [ ] Admin password is long and unique
- [ ] `PUBLIC_BASE_URL` is https (turns on Secure cookies and HSTS)
- [ ] Only ports 22, 80, 443 are open; SSH uses keys, not passwords
- [ ] `/metrics` returns 404 from the internet
- [ ] Backups run, are copied off the server, and a restore was tested
- [ ] Seed prices were replaced with your real prices in the admin page
- [ ] The privacy text and consent wording were reviewed (see `docs/PRIVACY.md`)
- [ ] `SEED_DEMO` is not set in production
