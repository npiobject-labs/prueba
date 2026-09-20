// Service worker de la app de notas: cachea la carcasa (HTML, manifest, iconos) para abrir
// sin red; la API de Fly es otro origen y no pasa por aquí. El nombre lleva el build para
// que cada publicación descarte la caché anterior.
const CACHE = 'notas-PR-B1-20260920-014';
const CARCASA = ['./', 'index.html', 'manifest.webmanifest', 'icono-192.png', 'icono-512.png'];

self.addEventListener('install', ev => {
  ev.waitUntil(caches.open(CACHE).then(c => c.addAll(CARCASA)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', ev => {
  ev.waitUntil(caches.keys().then(ks => Promise.all(ks.filter(k => k !== CACHE).map(k => caches.delete(k)))).then(() => self.clients.claim()));
});

// Mismo origen y GET: red primero (para coger builds nuevos) y caché si no hay red.
self.addEventListener('fetch', ev => {
  const url = new URL(ev.request.url);
  if (ev.request.method !== 'GET' || url.origin !== location.origin) return;
  ev.respondWith(
    fetch(ev.request).then(r => {
      if (r.ok) caches.open(CACHE).then(c => c.put(ev.request, r.clone()));
      return r;
    }).catch(() => caches.match(ev.request, { ignoreSearch: true }).then(r => r || caches.match('index.html')))
  );
});
