// sjbis-phone service worker: shows pushes as notifications and opens the ask on tap.
// Payloads come from the daemon (push_handlers.rs): {kind: 'ask'|'closed'|'test', id, title, body, ...}.
// Every push shows a notification: iOS drops the subscription of a web app that receives pushes
// and shows nothing, so a 'closed' push replaces the ask's banner (same tag) instead of just closing it.
'use strict';

self.addEventListener('install', function () { self.skipWaiting(); });
self.addEventListener('activate', function (e) { e.waitUntil(self.clients.claim()); });

function pageUrl(id) {
  var scope = self.registration.scope.replace(/\/+$/, '');
  return scope + '/phone' + (id ? '#' + encodeURIComponent(id) : '');
}

self.addEventListener('push', function (e) {
  var d = {};
  try { d = e.data ? e.data.json() : {}; } catch (_) { d = { kind: 'text', title: 'sjbis', body: e.data ? e.data.text() : '' }; }
  var tag = d.id || 'sjbis';
  var opts = { body: d.body || '', tag: tag, data: d, renotify: d.kind === 'ask' };
  if (d.kind === 'closed') opts.silent = true;
  var show = self.registration.showNotification(d.title || 'sjbis', opts);
  var tell = self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then(function (cs) {
    cs.forEach(function (c) { c.postMessage({ push: d }); });
  });
  e.waitUntil(Promise.all([show, tell]));
});

self.addEventListener('notificationclick', function (e) {
  e.notification.close();
  var d = e.notification.data || {};
  var id = d.kind === 'ask' || d.kind === 'closed' ? d.id : null;
  e.waitUntil(self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then(function (cs) {
    for (var i = 0; i < cs.length; i++) {
      if (cs[i].url.indexOf('/phone') >= 0 && 'focus' in cs[i]) {
        cs[i].postMessage({ open: id });
        return cs[i].focus();
      }
    }
    return self.clients.openWindow(pageUrl(id));
  }));
});
