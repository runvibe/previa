// Legacy service worker cleanup.
//
// Previa used to register a Workbox service worker at /sw.js. The embedded
// app is served by previa-main and must not keep stale bundles cached across
// local rebuilds, so this worker removes old caches and unregisters itself.
self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.map((key) => caches.delete(key)));
      await self.registration.unregister();

      const clients = await self.clients.matchAll({ type: "window" });
      await Promise.all(clients.map((client) => client.navigate(client.url)));
    })(),
  );
});
