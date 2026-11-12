# ADR 0008: Photo try-on provider (Tier B)

**Status:** accepted; revisit every few months, this market moves fast.

Researched on 2026-11-11. Prices and terms are from vendor pages and
third-party comparisons on that date; **confirm them before budgeting**.

| Option | Quality (reported) | Latency | Cost per image | Commercial use | Data handling |
|---|---|---|---|---|---|
| **FASHN API, `tryon-v1.6`** (hosted) | Strong on tops; keeps prints legible | about 5-20 s | $0.075 (1 credit; $7.50 minimum purchase) | Yes | Inline (base64) inputs deleted when the request finishes; CDN outputs deleted after 3 days; base64 outputs readable for 60 minutes; not used for training without opt-in |
| FASHN `tryon-max` (hosted) | Higher, up to 4K | slower | 1-5 credits | Yes | Same; labelled "Preview" by the vendor |
| Kling Kolors try-on via fal.ai | Good; 768x1024 output | about 10-30 s | about $0.07 | Yes (per fal listing) | Depends on fal and Kling terms; inputs go through two companies |
| Google Vertex AI Virtual Try-On | Good | about 10-20 s | about $0.06 (one secondary source) | Yes | Google Cloud terms; needs a GCP project and billing |
| Self-hosted FASHN VTON v1.5 (Apache-2.0 weights) | Good | 10-30 s on a rented 24 GB GPU | GPU rent, roughly $0.3-0.8/hour whether used or not | Yes (Apache-2.0) | Photos never leave your server |
| Self-hosted IDM-VTON / CatVTON / OOTDiffusion | Good | similar | GPU rent | **No**: non-commercial licences (CC BY-NC-SA) | n/a |
| Self-hosted Leffa | Good | similar | GPU rent | MIT, but check the weights' and training data's terms yourself | Photos never leave your server |

## Decision
Implement **FASHN `tryon-v1.6`** as the real provider, with the mock as
default. Reasons: per-image pricing with no idle cost suits low, bursty
volume; the documented data policy is the clearest of the hosted options;
the API is a simple submit-and-poll; commercial use is allowed.

The client sends both images inline as base64 and asks for a base64 result
(`return_base64: true`), so neither the customer's photo nor the result is
left on a public URL. Result URLs, if the API returns them anyway, are only
fetched from FASHN's CDN hosts.

Self-hosting FASHN VTON v1.5 is the planned alternative if volume grows
(roughly, when a GPU rented for the busy hours costs less than the API) or
if photos must not leave the country. It fits behind the same
`TryOnProvider` trait.

## What could not be verified from here
- The status endpoint is implemented as `GET /v1/status/{id}`. The docs
  name `/v1/status` and "poll using the prediction ID" but the fetched text
  did not show the exact path form. **Make one real call with your key
  (the eval harness does this) before enabling it for customers.**
- The exact shape of base64 output. The client accepts both a `data:` URI
  and a CDN URL in `output[0]`.
- No real generation was run: there is no API key in this environment.
  Quality, latency and cost columns above are reported figures, not
  measurements. `crates/evalkit` exists to measure them on your photos.

## Controls implemented around any provider
Consent before upload; resolution and brightness checks on the server;
the "exactly one person, torso visible" check on the device (it needs the
pose model, and running it there keeps rejected photos off the server; a
client that skips it only wastes its own daily quota); per-device and global daily limits; a daily cost
cap that counts queued jobs at full price; result caching for identical
photo + design; automatic deletion after `TRYON_TTL_HOURS`; "delete now".
