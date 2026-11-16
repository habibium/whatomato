// The service worker: shows what the server pushes, and opens the page a notification points at.
// It caches nothing, so the app itself always comes from the network.

self.addEventListener('install', () => self.skipWaiting())
self.addEventListener('activate', (event) =>
  event.waitUntil(self.clients.claim()),
)

self.addEventListener('push', (event) => {
  let pushed = {}
  try {
    pushed = event.data ? event.data.json() : {}
  } catch {
    // A push the server didn't shape still has to show something: browsers require it
  }
  const text = (value) => (typeof value === 'string' ? value : '')
  const tag = text(pushed.tag)
  const url = text(pushed.url)
  event.waitUntil(
    self.registration.showNotification(text(pushed.title) || 'Whatomato', {
      body: text(pushed.body),
      icon: '/favicon-192.png',
      badge: '/favicon-192.png',
      data: { url: url.startsWith('/') && !url.startsWith('//') ? url : '/' },
      // One tag keeps one notification; `renotify` makes each new one sound again
      ...(tag ? { tag, renotify: true } : {}),
    }),
  )
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const url = new URL(event.notification.data?.url ?? '/', self.location.origin)
    .href
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({
        type: 'window',
        includeUncontrolled: true,
      })
      const open = windows.find(
        (client) => new URL(client.url).origin === self.location.origin,
      )
      if (!open) return self.clients.openWindow(url)
      await open.focus()
      return open.navigate(url).catch(() => self.clients.openWindow(url))
    })(),
  )
})
