// Service worker: offline shell and caching of the large try-on files.
//
// Strategy by request type:
//   /api/*            never cached (orders, prices and photos must be live)
//   /vendor/, /fonts/ cache-first (big, versioned, rarely change)
//   fingerprinted     cache-first (the hash in the name changes on rebuild)
//   everything else   network-first, falling back to the cached copy, so a
//                     new deploy is picked up immediately when online
const CACHE = "qd-v1";
const SHELL = ["/", "/manifest.webmanifest", "/icons/icon-192.png"];

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))).then(() => self.clients.claim())
  );
});

const FINGERPRINT = /-[0-9a-f]{16,}(_bg)?\.[a-z0-9]+$/;

async function cacheFirst(request) {
  const cache = await caches.open(CACHE);
  const hit = await cache.match(request);
  if (hit) return hit;
  const res = await fetch(request);
  if (res.ok) cache.put(request, res.clone());
  return res;
}

async function networkFirst(request, fallbackUrl) {
  const cache = await caches.open(CACHE);
  try {
    const res = await fetch(request);
    if (res.ok) cache.put(request, res.clone());
    return res;
  } catch (e) {
    const hit = (await cache.match(request)) || (fallbackUrl && (await cache.match(fallbackUrl)));
    if (hit) return hit;
    throw e;
  }
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/admin") || url.pathname === "/metrics") return;

  if (url.pathname.startsWith("/vendor/") || url.pathname.startsWith("/fonts/") || FINGERPRINT.test(url.pathname)) {
    event.respondWith(cacheFirst(req));
  } else if (req.mode === "navigate") {
    // Any client-side route falls back to the cached app shell when offline.
    event.respondWith(networkFirst(req, "/"));
  } else {
    event.respondWith(networkFirst(req));
  }
});
