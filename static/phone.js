// SJBIS phone: list of open asks, one ask at a time to answer, installed to the iOS Home Screen.
// Plain asks POST /answer/{id}. Approval asks are signed here with a WebCrypto P-256 key kept in
// IndexedDB (non-extractable) and posted as a ta envelope to /grants; the daemon accepts it once
// the key is pinned in its signers.toml. Push: the service worker (phone-sw.js) shows daemon pushes.
(function () {
  'use strict';

  var C = window.SjbisPhoneCrypto;
  var BASE = window.__SJBIS_BASE_PATH__ != null ? window.__SJBIS_BASE_PATH__
    : window.location.pathname.replace(/\/phone(\.html)?\/?$/, '').replace(/\/+$/, '');
  var PRESET_TTLS = [[600, '10m'], [3600, '1h'], [14400, '4h'], [86400, '24h']];
  var SNOOZE_MINUTES = 15;
  var OK_MSG_MS = 4000;
  var DISMISS_ARM_MS = 3000;
  var DEFAULT_SIGNER = 'phone@iphone';
  var FORM_SHAPES = { yesno: 'yesno', numeric: 'numeric', freetext: 'freetext', text: 'freetext', ack: 'ack', multichoice: 'multichoice' };

  var S = {
    queue: [], view: 'list', cur: null, sel: null, ttlIdx: 1, once: false, busy: false, msg: '', msgCls: '', msgAt: 0,
    conn: 'connecting', dev: null, enrolled: null, signers: null, grant: null, closed: null,
    push: { supported: false, permission: 'default', subscribed: false, standalone: false, vapid: null, enabled: null },
    formIdx: 0, formAnswers: {}, num: 0, dismissArm: null, dismissArmUntil: 0, noteOpen: false,
  };
  var root = document.getElementById('phone');
  if (!root) { root = document.createElement('div'); root.id = 'phone'; document.body.appendChild(root); }
  var $ = function (id) { return document.getElementById(id); };

  // ── helpers ────────────────────────────────────────────────────────────
  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }
  function inline(s) {
    return esc(s).replace(/`([^`]+)`/g, '<code>$1</code>').replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  }
  function md(src) {
    var out = [], para = [], list = null, code = null;
    function flush() {
      if (para.length) { out.push('<p>' + inline(para.join(' ')) + '</p>'); para = []; }
      if (list) { out.push('<ul>' + list.join('') + '</ul>'); list = null; }
    }
    String(src).split(/\r?\n/).forEach(function (line) {
      var m;
      if (code) {
        if (/^\s*```/.test(line)) { out.push('<pre>' + esc(code.join('\n')) + '</pre>'); code = null; } else code.push(line);
      } else if (/^\s*```/.test(line)) { flush(); code = []; }
      else if ((m = line.match(/^(#{1,6})\s+(.*)$/))) {
        flush(); var h = Math.min(m[1].length + 1, 4); out.push('<h' + h + '>' + inline(m[2]) + '</h' + h + '>');
      } else if ((m = line.match(/^\s*[-*+]\s+(.*)$/))) {
        if (para.length) { out.push('<p>' + inline(para.join(' ')) + '</p>'); para = []; }
        (list = list || []).push('<li>' + inline(m[1]) + '</li>');
      } else if (!line.trim()) flush();
      else { if (list) flush(); para.push(line.replace(/^\s*>\s?/, '').trim()); }
    });
    if (code) out.push('<pre>' + esc(code.join('\n')) + '</pre>');
    flush();
    return out.join('');
  }
  function fmtAge(ts, nowMs, suffix) {
    return window.SjbisTime ? window.SjbisTime.formatAge(ts, { nowMs: nowMs, suffix: suffix }) : '';
  }
  function fmtTtl(secs) {
    if (secs % 86400 === 0) return (secs / 86400) + 'd';
    if (secs % 3600 === 0) return (secs / 3600) + 'h';
    if (secs % 60 === 0) return (secs / 60) + 'm';
    return secs + 's';
  }
  function ttls(n) {
    var t = n && n.ttl_secs;
    if (!t || PRESET_TTLS.some(function (p) { return p[0] === t; })) return PRESET_TTLS;
    return [[t, fmtTtl(t) + ' (asked)']].concat(PRESET_TTLS);
  }
  function current() { for (var i = 0; i < S.queue.length; i++) if (S.queue[i].id === S.cur) return S.queue[i]; return null; }
  function isApproval(n) { return !!n && n.question_type === 'approval'; }
  function isNumeric(n) { return !!n && n.question_type === 'numeric'; }
  function isForm(n) { return !!(n && n.question_type === 'form' && Array.isArray(n.sub_questions) && n.sub_questions.length); }
  function formSub(n) { return isForm(n) ? n.sub_questions[Math.min(S.formIdx || 0, n.sub_questions.length - 1)] : null; }
  function unit(n) {
    var sq = formSub(n);
    if (!sq) return n;
    return { question_type: FORM_SHAPES[sq.shape] || 'unsupported', question: sq.question, choices: sq.choices, min: sq.min, max: sq.max,
      step: sq.step, default_value: sq['default'], unit: sq.unit, ack_label: sq.ack_label, key: sq.key };
  }
  function numStart(n) {
    if (!isNumeric(n)) return 0;
    if (typeof n.default_value === 'number') return n.default_value;
    return typeof n.min === 'number' ? n.min : 0;
  }
  function numStep(n) { return typeof n.step === 'number' && n.step > 0 ? n.step : 1; }
  function numFmt(v) { return String(Math.round(v * 1e6) / 1e6); }
  function numClamp(n, v) {
    if (typeof n.min === 'number') v = Math.max(n.min, v);
    if (typeof n.max === 'number') v = Math.min(n.max, v);
    return v;
  }
  function numRange(n) {
    var lo = typeof n.min === 'number' ? numFmt(n.min) : '', hi = typeof n.max === 'number' ? numFmt(n.max) : '';
    return lo || hi ? lo + '–' + hi : 'any number';
  }
  function options(n) {
    if (!n) return [];
    if (n.choices && n.choices.length) {
      return n.choices.map(function (c) { return typeof c === 'string' ? { value: c, label: c } : { value: c.value, label: c.label || c.value }; });
    }
    if (n.question_type === 'picklist' && n.items) return n.items.map(function (it) { return { value: it.title, label: it.title, meta: it.meta }; });
    if (n.question_type === 'schedule' && n.slots) {
      return n.slots.map(function (s) { var v = s.day + ' · ' + s.time; return { value: v, label: v }; });
    }
    return [];
  }
  function setMsg(text, cls) { S.msg = text || ''; S.msgCls = cls || ''; S.msgAt = Date.now(); }
  function noteText() { var el = $('p-note'); var v = el ? el.value.trim() : ''; return v || null; }
  function textText() { var el = $('p-text'); var v = el ? el.value.trim() : ''; return v || null; }

  // ── device key (IndexedDB) ─────────────────────────────────────────────
  var idb = {
    open: function () {
      return new Promise(function (res, rej) {
        var r = indexedDB.open('sjbis-phone', 1);
        r.onupgradeneeded = function () { r.result.createObjectStore('kv'); };
        r.onsuccess = function () { res(r.result); };
        r.onerror = function () { rej(r.error); };
      });
    },
    get: function (k) {
      return idb.open().then(function (db) {
        return new Promise(function (res, rej) {
          var r = db.transaction('kv').objectStore('kv').get(k);
          r.onsuccess = function () { res(r.result); }; r.onerror = function () { rej(r.error); };
        });
      });
    },
    set: function (k, v) {
      return idb.open().then(function (db) {
        return new Promise(function (res, rej) {
          var tx = db.transaction('kv', 'readwrite');
          tx.objectStore('kv').put(v, k);
          tx.oncomplete = function () { res(); }; tx.onerror = function () { rej(tx.error); };
        });
      });
    },
  };
  function randId() {
    var b = new Uint8Array(12); crypto.getRandomValues(b);
    return C.b64url(b);
  }
  function guessName() {
    var ua = navigator.userAgent || '';
    if (/iPhone/.test(ua)) return 'iPhone';
    if (/iPad/.test(ua)) return 'iPad';
    if (/Android/.test(ua)) return 'Android phone';
    if (/Macintosh/.test(ua)) return 'Mac browser';
    return 'phone';
  }
  function loadDevice() {
    return idb.get('device').then(function (d) {
      if (d && d.keyPair && d.pubkey) return d;
      return C.generateKeyPair().then(function (kp) {
        return C.exportPubkey(kp.publicKey).then(function (pub) {
          var dev = { id: randId(), signerId: DEFAULT_SIGNER, name: guessName(), keyPair: kp, pubkey: pub, created: new Date().toISOString() };
          return idb.set('device', dev).then(function () { return dev; });
        });
      });
    }).then(function (d) {
      S.dev = d;
      if (navigator.storage && navigator.storage.persist) navigator.storage.persist().catch(function () {});
      return d;
    });
  }
  function saveDevice() { return idb.set('device', S.dev); }

  // ── network ────────────────────────────────────────────────────────────
  function readJson(r) {
    return r.json().catch(function () { return {}; }).then(function (j) {
      if (!r.ok) {
        var e = new Error((j.error || 'HTTP ' + r.status) + (j.code ? ' [' + j.code + ']' : ''));
        e.status = r.status; e.code = j.code; throw e;
      }
      return j;
    });
  }
  function post(url, body) {
    return fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body || {}) }).then(readJson);
  }
  function load() {
    return fetch(BASE + '/list', { cache: 'no-store' }).then(readJson).then(function (list) {
      S.queue = Array.isArray(list) ? list : [];
      normalize(); render();
    }).catch(function (e) { setMsg('load failed: ' + e.message, 'bad'); render(); });
  }
  function checkEnrolment() {
    return fetch(BASE + '/signers', { cache: 'no-store' }).then(readJson).then(function (list) {
      S.signers = Array.isArray(list) ? list : [];
      var me = S.signers.filter(function (s) { return s.id === S.dev.signerId; })[0];
      S.enrolled = !!(me && me.pubkey && me.pubkey.trim() === S.dev.pubkey);
      S.enrolClash = !!(me && !S.enrolled);
    }).catch(function () { S.enrolled = null; }).then(render);
  }
  function register(subscription) {
    var body = { device_id: S.dev.id, device_name: S.dev.name, signer_id: S.dev.signerId, pubkey: S.dev.pubkey, user_agent: navigator.userAgent };
    if (subscription) body.subscription = subscription;
    return post(BASE + '/push/register', body).then(function (j) {
      S.registered = true;
      if (typeof j.enrolled === 'boolean') S.enrolled = j.enrolled;
      return j;
    });
  }
  var es = null, backoff = 1000, everOpen = false;
  function connect() {
    if (es) es.close();
    es = new EventSource(BASE + '/events');
    es.onopen = function () {
      backoff = 1000; S.conn = 'live';
      if (everOpen) load(); else render();
      everOpen = true;
    };
    es.onmessage = function (e) {
      var ev;
      try { ev = JSON.parse(e.data); } catch (_) { return; }
      if (ev.event === 'notification_created' || ev.event === 'notification_updated') upsert(ev.notification);
      else if (ev.event === 'notification_answered') closedElsewhere(ev.envelope && ev.envelope.id, ev.envelope);
      else if (ev.event === 'notification_cancelled' || ev.event === 'notification_dismissed') closedElsewhere(ev.id, null);
    };
    es.onerror = function () {
      if (es) es.close();
      es = null; everOpen = true; S.conn = 'reconnecting'; render();
      setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, 30000);
    };
  }

  // ── queue ──────────────────────────────────────────────────────────────
  function live(n, now) {
    return n && (!n.status || n.status === 'open')
      && !(n.snooze_until && Date.parse(n.snooze_until) > now)
      && !(n.deadline && Date.parse(n.deadline) <= now);
  }
  function normalize() {
    var now = Date.now();
    var keep = S.queue.filter(function (n) { return live(n, now); });
    keep.sort(function (a, b) { return (b.urgency || 0) - (a.urgency || 0) || Date.parse(a.created_at) - Date.parse(b.created_at); });
    S.queue = keep;
    if (S.view === 'ask' && !current()) { S.view = 'list'; S.cur = null; }
  }
  function upsert(n) {
    S.queue = S.queue.filter(function (q) { return q.id !== n.id; }).concat([n]);
    normalize(); render();
  }
  function removeAsk(id) {
    S.queue = S.queue.filter(function (n) { return n.id !== id; });
    if (S.cur === id) { S.cur = null; if (S.view === 'ask') S.view = 'list'; }
    render();
  }
  // Answered, cancelled or dismissed somewhere else while we were looking at it.
  function closedElsewhere(id, envelope) {
    if (!id) return;
    if (S.view === 'ask' && S.cur === id && !S.busy) {
      var n = current();
      S.closed = { id: id, question: n ? n.question : '', what: envelope ? 'answered: ' + (envelope.answer_label || envelope.answer || '') + (envelope.via ? ' (via ' + envelope.via + ')' : '') : 'closed' };
    }
    removeAsk(id);
    closeBanners(id);
  }
  function closeBanners(id) {
    if (!navigator.serviceWorker || !navigator.serviceWorker.getRegistration) return;
    navigator.serviceWorker.getRegistration().then(function (reg) {
      if (!reg || !reg.getNotifications) return;
      return reg.getNotifications({ tag: id }).then(function (ns) { ns.forEach(function (x) { x.close(); }); });
    }).catch(function () {});
  }
  function open(id) {
    var n = null;
    S.queue.forEach(function (q) { if (q.id === id) n = q; });
    if (!n) {
      // Not in the open list (maybe already answered): fetch it to say so.
      return fetch(BASE + '/notification/' + encodeURIComponent(id), { cache: 'no-store' }).then(readJson).then(function (full) {
        if (live(full, Date.now())) { upsert(full); return open(id); }
        S.closed = { id: id, question: full.question || '', what: full.status === 'answered' ? 'answered: ' + (full.answer_label || full.answer || '') : full.status || 'closed' };
        S.view = 'list'; render();
      }).catch(function (e) { setMsg(id + ': ' + e.message, 'bad'); S.view = 'list'; render(); });
    }
    S.cur = id; S.view = 'ask'; S.sel = null; S.noteOpen = false; setMsg('');
    S.dismissArm = null; S.dismissArmUntil = 0;
    S.formIdx = 0; S.formAnswers = {};
    S.num = numStart(unit(n));
    S.once = n.once === true;
    var list = ttls(n); S.ttlIdx = 1;
    list.forEach(function (p, i) { if (p[0] === n.ttl_secs) S.ttlIdx = i; });
    render();
    window.scrollTo(0, 0);
    return Promise.resolve();
  }
  function back() { S.view = 'list'; S.cur = null; S.closed = null; setMsg(''); render(); window.scrollTo(0, 0); }

  // ── actions ────────────────────────────────────────────────────────────
  function run(promise, okText) {
    S.busy = true; setMsg('working…'); render();
    return promise.then(function () { setMsg(okText || '', 'ok'); })
      .catch(function (e) { setMsg(e.message || String(e), 'bad'); })
      .then(function () { S.busy = false; render(); });
  }
  function answer(n, value) {
    var body = { answer: value, via: 'phone', renderer: 'phone' };
    var note = noteText();
    if (note) body.note = note;
    return run(post(BASE + '/answer/' + encodeURIComponent(n.id), body).then(function () {
      S.lastAnswer = { id: n.id, value: value, question: n.question };
      removeAsk(n.id); closeBanners(n.id);
    }), 'answered: ' + value);
  }
  function submit(n, value) {
    if (!isForm(n)) return answer(n, value);
    S.formAnswers[formSub(n).key] = value;
    if (S.formIdx + 1 < n.sub_questions.length) { S.formIdx++; S.sel = null; S.num = numStart(unit(n)); setMsg(''); render(); return; }
    return answer(n, JSON.stringify(S.formAnswers));
  }
  // Sign the grant here and post it; retry once on an id collision with a fresh issued_at.
  function approve(n, decision, value) {
    if (!S.dev) { setMsg('no signing key yet', 'bad'); return render(); }
    if (S.enrolled === false) { setMsg('this phone’s key is not pinned on the daemon yet (Settings)', 'bad'); return render(); }
    var ttl = ttls(n)[S.ttlIdx][0];
    var opts = { decision: decision, answer: decision === 'approve' ? value : null, note: noteText(), ttlSecs: ttl, once: S.once || n.once === true, signer: S.dev.signerId };
    function attempt(k) {
      var rec = C.grantRecord(n, opts);
      return C.signRecord(rec, S.dev.keyPair.privateKey).then(function (env) {
        return post(BASE + '/grants', env).then(function () { return env; }).catch(function (e) {
          if (k < 2 && (e.status === 409 || /collision/i.test(e.message))) {
            return new Promise(function (r) { setTimeout(r, 1100); }).then(function () { return attempt(k + 1); });
          }
          throw e;
        });
      });
    }
    return run(attempt(0).then(function (env) {
      S.grant = { id: env.id, expires_at: env.record.expires_at, decision: decision, question: n.question, answer: env.record.answer };
      removeAsk(n.id); closeBanners(n.id);
    }));
  }
  function act(name, arg) {
    var n = current();
    if (name !== 'dismiss' && S.dismissArm != null) { S.dismissArm = null; if (S.msg === 'tap dismiss again to confirm') setMsg(''); }
    if (!n || S.busy) return;
    var u = unit(n), t = u.question_type, opts = options(u);
    if (name === 'pick') { S.sel = (S.sel === arg) ? null : arg; setMsg(''); return render(); }
    if (name === 'ttl') { S.ttlIdx = arg; return render(); }
    if (name === 'once') { if (n.once !== true) S.once = !S.once; return render(); }
    if (name === 'note') { S.noteOpen = !S.noteOpen; render(); if (S.noteOpen && $('p-note')) $('p-note').focus(); return; }
    if (name === 'back-step') { if (isForm(n) && S.formIdx > 0) { S.formIdx--; S.sel = null; S.num = numStart(unit(n)); render(); } return; }
    if (name === 'snooze') {
      var mins = SNOOZE_MINUTES;
      if (n.deadline) mins = Math.min(mins, Math.floor((Date.parse(n.deadline) - Date.now()) / 60000));
      if (mins < 1) { setMsg('deadline too close to snooze', 'bad'); return render(); }
      return run(post(BASE + '/snooze/' + encodeURIComponent(n.id), { minutes: mins }).then(function () { removeAsk(n.id); }), 'snoozed ' + mins + 'm');
    }
    if (name === 'dismiss') {
      if (!(S.dismissArm === n.id && Date.now() < S.dismissArmUntil)) {
        S.dismissArm = n.id; S.dismissArmUntil = Date.now() + DISMISS_ARM_MS;
        setMsg('tap dismiss again to confirm', 'warn');
        setTimeout(function () { if (S.dismissArm === n.id && Date.now() >= S.dismissArmUntil) { S.dismissArm = null; if (S.msg === 'tap dismiss again to confirm') setMsg(''); render(); } }, DISMISS_ARM_MS + 50);
        return render();
      }
      S.dismissArm = null;
      return run(post(BASE + '/dismiss/' + encodeURIComponent(n.id)).then(function () { removeAsk(n.id); closeBanners(n.id); }), 'dismissed');
    }
    if (t === 'unsupported') { setMsg('this form has a sub-question the phone cannot answer; open it on the dashboard', 'bad'); return render(); }
    if (isApproval(n)) {
      if (name === 'no') return approve(n, 'deny', null);
      if (name === 'yes') {
        if (opts.length && S.sel == null) { setMsg('pick an option first', 'bad'); return render(); }
        return approve(n, 'approve', opts.length ? opts[S.sel].value : null);
      }
      return;
    }
    if (isNumeric(u)) {
      if (name === 'minus' || name === 'plus') { S.num = numClamp(u, S.num + (name === 'plus' ? 1 : -1) * numStep(u)); setMsg(''); return render(); }
      if (name === 'confirm') {
        var typed = textText(), v = typed ? Number(typed.replace(/,/g, '')) : S.num;
        if (typed && !isFinite(v)) { setMsg('"' + typed + '" is not a number', 'bad'); return render(); }
        if ((typeof u.min === 'number' && v < u.min) || (typeof u.max === 'number' && v > u.max)) { setMsg(numFmt(v) + ' is outside ' + numRange(u), 'bad'); return render(); }
        return submit(n, isForm(n) ? numFmt(v) : numFmt(v) + (u.unit ? ' ' + u.unit : ''));
      }
      return;
    }
    if (name === 'yes') {
      if (t === 'yesno') return submit(n, u.yes_label || 'Yes');
      if (t === 'diff') return submit(n, 'Approved');
      if (t === 'ack') return submit(n, isForm(n) ? 'acknowledged' : u.ack_label || 'Acknowledged');
    }
    if (name === 'no') {
      if (t === 'yesno') return submit(n, u.no_label || 'No');
      if (t === 'diff') return submit(n, 'Rejected');
    }
    if (name === 'confirm') {
      if (S.sel != null && opts[S.sel]) return submit(n, opts[S.sel].value);
      if (t === 'freetext') { var text = textText(); if (!text) { setMsg('type an answer first', 'bad'); return render(); } return submit(n, text); }
      if (t === 'ack') return submit(n, isForm(n) ? 'acknowledged' : u.ack_label || 'Acknowledged');
      if (opts.length) { setMsg('pick an option first', 'bad'); return render(); }
    }
  }

  // ── push ───────────────────────────────────────────────────────────────
  function pushState() {
    var p = S.push;
    p.standalone = window.navigator.standalone === true || (window.matchMedia && window.matchMedia('(display-mode: standalone)').matches);
    p.supported = !!(window.PushManager && navigator.serviceWorker && window.Notification);
    p.permission = window.Notification ? Notification.permission : 'unsupported';
    p.ios = /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
  }
  function swRegister() {
    if (!navigator.serviceWorker) return Promise.resolve(null);
    return navigator.serviceWorker.register(BASE + '/phone-sw.js').catch(function (e) { setMsg('service worker: ' + e.message, 'bad'); return null; });
  }
  function fetchVapid() {
    return fetch(BASE + '/push/vapid', { cache: 'no-store' }).then(readJson).then(function (j) {
      S.push.enabled = j.enabled === true; S.push.vapid = j.public_key || null;
    }).catch(function () { S.push.enabled = null; });
  }
  function currentSubscription() {
    if (!navigator.serviceWorker) return Promise.resolve(null);
    return navigator.serviceWorker.getRegistration().then(function (reg) {
      return reg && reg.pushManager ? reg.pushManager.getSubscription() : null;
    }).then(function (sub) { S.push.subscribed = !!sub; return sub; }).catch(function () { return null; });
  }
  function enablePush() {
    pushState();
    if (!S.push.supported) { setMsg(S.push.ios && !S.push.standalone ? 'add this page to your Home Screen first, then open it from there' : 'this browser has no Web Push', 'bad'); return render(); }
    if (!S.push.vapid) { setMsg('the daemon has no VAPID key (web push disabled)', 'bad'); return render(); }
    S.busy = true; setMsg('asking for permission…'); render();
    return Notification.requestPermission().then(function (perm) {
      S.push.permission = perm;
      if (perm !== 'granted') throw new Error('notifications are ' + perm);
      return navigator.serviceWorker.ready;
    }).then(function (reg) {
      return reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: C.unb64(S.push.vapid) });
    }).then(function (sub) {
      S.push.subscribed = true;
      return register(sub.toJSON());
    }).then(function () { setMsg('notifications on', 'ok'); })
      .catch(function (e) { setMsg('push: ' + (e.message || e), 'bad'); })
      .then(function () { S.busy = false; render(); });
  }
  function testPush() {
    return run(post(BASE + '/push/test', { device_id: S.dev.id }).then(function (j) {
      var r = (j.results || []).filter(function (x) { return x.device_id === S.dev.id; })[0];
      if (!r || r.result !== 'sent') throw new Error(r ? r.result : 'this device is not registered');
    }), 'test push sent; it should arrive in a moment');
  }
  function onSwMessage(e) {
    var d = e.data || {};
    if (d.open) { open(d.open); return; }
    if (d.push && d.push.kind === 'ask') load();
    if (d.push && d.push.kind === 'closed') closedElsewhere(d.push.id, null);
  }
  function copy(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text).then(function () { setMsg('copied', 'ok'); render(); }, function () { setMsg('copy failed; long-press the text instead', 'bad'); render(); });
    }
    setMsg('copy not available; long-press the text instead', 'bad'); render();
  }

  // ── render ─────────────────────────────────────────────────────────────
  function btn(act, label, cls, disabled, arg, title) {
    return '<button type="button" data-act="' + act + '"' + (arg != null ? ' data-arg="' + esc(arg) + '"' : '') + (title ? ' title="' + esc(title) + '"' : '')
      + ' class="' + (cls || '') + '"' + (disabled ? ' disabled' : '') + '><span class="lbl">' + label + '</span></button>';
  }
  function pills() {
    var h = [];
    h.push('<span class="pill ' + (S.conn === 'live' ? 'ok' : 'warn') + '">' + esc(S.conn) + '</span>');
    if (S.enrolled === true) h.push('<span class="pill ok" title="signing key pinned">\u{1F512} ' + esc(S.dev.signerId) + '</span>');
    else if (S.enrolled === false) h.push('<span class="pill bad">key not pinned</span>');
    if (S.push.subscribed && S.push.permission === 'granted') h.push('<span class="pill ok">\u{1F514}</span>');
    return h.join('');
  }
  function renderList() {
    var h = ['<div class="top"><span class="title">sjbis</span><span class="sp"></span>' + pills()
      + '<button type="button" class="iconbtn" data-nav="settings" aria-label="settings">⚙︎</button></div>'];
    h.push('<div class="list">');
    if (S.closed) {
      h.push('<div class="lock warn">' + esc(S.closed.id) + ' is no longer open: ' + esc(S.closed.what)
        + (S.closed.question ? '<div class="dim" style="font-weight:400;margin-top:4px">' + esc(S.closed.question.slice(0, 160)) + '</div>' : '')
        + '<div style="margin-top:8px">' + btn('closed-ok', 'ok', 'ghost') + '</div></div>');
    }
    if (S.msg) h.push('<div class="msg ' + S.msgCls + '">' + esc(S.msg) + '</div>');
    if (!S.queue.length) {
      h.push('<div class="empty"><div class="big">✓</div><div>no open asks</div><div class="' + (S.conn === 'live' ? 'ok' : 'warn') + '">daemon ' + esc(S.conn) + '</div>'
        + (S.enrolled === false ? '<div class="bad">this phone’s signing key is not pinned yet → Settings</div>' : '') + '</div>');
    } else {
      h.push('<div class="section">' + S.queue.length + ' open</div>');
      S.queue.forEach(function (n) {
        var appr = isApproval(n);
        h.push('<button type="button" class="card u' + esc(n.urgency) + (appr ? ' appr' : '') + '" data-open="' + esc(n.id) + '">'
          + '<div class="meta"><b>' + esc(n.agent_name) + '</b><span class="sp"></span>'
          + '<span class="chip' + (appr ? ' appr' : '') + '">' + esc(appr ? 'approval' : n.question_type) + '</span>'
          + (n.urgency >= 4 ? '<span class="chip" style="background:var(--bad);color:#000">U' + esc(n.urgency) + '</span>' : '')
          + '<span class="age" data-ts="' + esc(n.created_at) + '">' + esc(fmtAge(n.created_at, Date.now())) + '</span></div>'
          + '<div class="q">' + esc(n.question) + '</div></button>');
      });
    }
    h.push('</div>');
    return h.join('');
  }
  function renderAsk(n) {
    var u = unit(n), appr = isApproval(n), opts = options(u), t = u.question_type, idx = 0;
    S.queue.forEach(function (q, j) { if (q.id === n.id) idx = j; });
    var dis = S.busy, h = [];
    h.push('<div class="top"><button type="button" class="iconbtn" data-nav="list" aria-label="back">‹</button>'
      + '<span class="title">' + esc(n.agent_name) + '</span><span class="sp"></span><span class="pill">' + (idx + 1) + ' of ' + S.queue.length + '</span></div>');
    h.push('<div class="ask"><div class="body">');
    h.push('<div class="meta row1"><span>' + esc(n.src) + '</span><span class="urg' + (n.urgency >= 4 ? ' hot' : '') + '">U' + esc(n.urgency) + '</span>'
      + '<span class="age" data-ts="' + esc(n.created_at) + '">' + esc(fmtAge(n.created_at, Date.now())) + '</span>'
      + (n.deadline ? '<span class="warn">deadline ' + esc(fmtAge(Date.now(), Date.parse(n.deadline), '')) + '</span>' : '') + '</div>');
    h.push('<h1 class="q' + (n.question.length > 300 ? ' xlong' : n.question.length > 110 ? ' long' : '') + '" id="p-q">' + esc(n.question) + '</h1>');
    if (n.detail_markdown) h.push('<div class="detail" id="p-detail">' + md(n.detail_markdown) + '</div>');
    else if (n.detail) h.push('<div class="detail plain" id="p-detail">' + esc(n.detail) + '</div>');
    if (t === 'diff' && Array.isArray(n.diff) && n.diff.length) {
      h.push('<div class="diff">' + n.diff.map(function (l) {
        var kind = l.kind || l.type || (typeof l === 'string' ? (l[0] === '+' ? 'add' : l[0] === '-' ? 'del' : 'ctx') : 'ctx');
        var text = typeof l === 'string' ? l : (l.text != null ? l.text : l.line || '');
        return '<div class="' + esc(kind) + '">' + esc(text) + '</div>';
      }).join('') + '</div>');
    }
    if (isForm(n)) h.push('<div class="formq"><span class="step">' + (S.formIdx + 1) + ' / ' + n.sub_questions.length + '</span>' + esc(formSub(n).question) + '</div>');
    if (appr) {
      if (S.enrolled === false) h.push('<div class="lock bad">\u{1F512} signed approval · this phone’s key is not pinned on the daemon yet (Settings)</div>');
      else h.push('<div class="lock ok">\u{1F512} signed approval · signer ' + esc(S.dev ? S.dev.signerId : '?') + (n.once === true ? ' · single-use' : '') + '</div>');
    }
    if (opts.length) {
      h.push('<div class="opts">' + opts.map(function (o, i) {
        return btn('pick', esc(o.label) + (o.meta ? ' <span class="dim">' + esc(o.meta) + '</span>' : ''), S.sel === i ? 'sel' : '', dis, i);
      }).join('') + '</div>');
    }
    if (isNumeric(u)) {
      h.push('<div class="num">' + btn('minus', '−', '', dis) + '<div class="val" id="p-num">' + esc(numFmt(S.num)) + (u.unit ? '<span class="unit">' + esc(u.unit) + '</span>' : '') + '</div>' + btn('plus', '+', '', dis) + '</div>');
      h.push('<div class="range">' + esc(numRange(u)) + ' · step ' + esc(numFmt(numStep(u))) + '</div>');
      h.push('<input type="text" id="p-text" inputmode="decimal" placeholder="or type a number" autocomplete="off">');
    }
    if (t === 'freetext') {
      h.push('<textarea id="p-text" rows="3" placeholder="' + esc(u.placeholder || 'type your answer') + '"></textarea>');
      if (Array.isArray(n.suggestions) && n.suggestions.length) {
        h.push('<div class="ttl">' + n.suggestions.slice(0, 6).map(function (s) { return btn('suggest', esc(s), '', dis, s); }).join('') + '</div>');
      }
    }
    if (appr) {
      h.push('<div class="ttl"><span>grant lasts</span>' + ttls(n).map(function (p, i) { return btn('ttl', esc(p[1]), S.ttlIdx === i ? 'sel' : '', dis, i); }).join('') + '</div>');
      h.push('<label class="toggle"><input type="checkbox" data-act="once"' + (S.once ? ' checked' : '') + (n.once === true ? ' disabled' : '') + '> single-use grant (claimable once)</label>');
    }
    h.push('<div class="toggle"><button type="button" class="ghost" data-act="note" style="min-height:40px;font-weight:500;color:var(--dim)">' + (S.noteOpen ? '− note' : '+ add a note') + '</button></div>');
    if (S.noteOpen) h.push('<textarea id="p-note" rows="2" placeholder="optional note, signed with an approval"></textarea>');
    h.push('<div class="msg ' + S.msgCls + '" id="p-msg">' + esc(S.msg) + '</div>');
    h.push('</div>'); // body
    // sticky bar
    var row = [];
    if (t === 'unsupported') row.push('<div class="lock warn">open this ask on the dashboard</div>');
    else if (appr) {
      row.push(btn('no', 'deny', 'no', dis));
      row.push(btn('yes', opts.length ? 'approve choice' : 'approve', 'yes', dis || (opts.length && S.sel == null)));
    } else if (t === 'yesno' || t === 'diff') {
      row.push(btn('no', esc(t === 'diff' ? 'Reject' : u.no_label || 'No'), 'no', dis));
      row.push(btn('yes', esc(t === 'diff' ? 'Approve' : u.yes_label || 'Yes'), 'yes', dis));
    } else if (t === 'ack') {
      row.push(btn('yes', esc(u.ack_label || 'Acknowledged'), 'yes', dis));
    } else if (isNumeric(u)) {
      row.push(btn('confirm', 'send', 'yes', dis));
    } else if (t === 'freetext') {
      row.push(btn('confirm', 'send', 'yes', dis));
    } else if (opts.length) {
      row.push(btn('confirm', 'confirm', 'yes', dis || S.sel == null));
    } else {
      row.push('<div class="lock warn">open this ask on the dashboard to answer it</div>');
    }
    h.push('<div class="bar"><div class="row">' + row.join('') + '</div><div class="row tools">'
      + (isForm(n) && S.formIdx > 0 ? btn('back-step', '⌫ back a step', '', dis) : '')
      + btn('snooze', 'snooze ' + SNOOZE_MINUTES + 'm', '', dis)
      + btn('dismiss', S.dismissArm === n.id ? 'dismiss?' : 'dismiss', S.dismissArm === n.id ? 'arm' : '', dis) + '</div></div>');
    h.push('</div>');
    return h.join('');
  }
  function renderSettings() {
    pushState();
    var p = S.push, d = S.dev, h = [];
    h.push('<div class="top"><button type="button" class="iconbtn" data-nav="list" aria-label="back">‹</button><span class="title">Settings</span></div>');
    h.push('<div class="settings">');
    if (S.msg) h.push('<div class="msg ' + S.msgCls + '">' + esc(S.msg) + '</div>');
    // Notifications
    h.push('<div class="kv"><div class="k">Notifications</div>');
    if (!p.standalone && p.ios) {
      h.push('<div class="v warn">Not installed yet.</div><ol class="steps"><li>Tap Share in Safari.</li><li>Choose <b>Add to Home Screen</b>.</li><li>Open sjbis from the new icon and come back here.</li></ol>'
        + '<div class="hint">iOS only delivers Web Push to web apps opened from the Home Screen.</div>');
    } else if (!p.supported) {
      h.push('<div class="v bad">This browser has no Web Push.</div>');
    } else if (p.permission === 'denied') {
      h.push('<div class="v bad">Notifications are blocked for this app. Allow them in iOS Settings → Notifications → sjbis.</div>');
    } else if (p.subscribed && p.permission === 'granted') {
      h.push('<div class="v ok">On · this device receives every new ask.</div>'
        + '<div class="row" style="display:flex;gap:8px;margin-top:6px">' + btn('test-push', 'send a test push', '', S.busy) + btn('enable-push', 're-subscribe', 'ghost', S.busy) + '</div>');
    } else {
      h.push('<div class="v">Off.</div><div style="margin-top:6px">' + btn('enable-push', 'enable notifications', 'yes', S.busy || p.enabled === false) + '</div>'
        + (p.enabled === false ? '<div class="hint bad">the daemon has web push disabled (no VAPID key)</div>' : ''));
    }
    h.push('</div>');
    // Signing key
    h.push('<div class="kv"><div class="k">Signing key</div>');
    if (!d) h.push('<div class="v dim">generating…</div>');
    else {
      h.push('<div class="v">signer id <code>' + esc(d.signerId) + '</code> · ' + (S.enrolled === true ? '<span class="ok">pinned on the daemon ✓</span>'
        : S.enrolled === false ? '<span class="bad">' + (S.enrolClash ? 'a different key is pinned under this id' : 'not pinned yet') + '</span>' : '<span class="dim">checking…</span>') + '</div>');
      h.push('<div class="v mono" id="p-pubkey">' + esc(d.pubkey) + '</div>');
      h.push('<div class="hint">P-256 key generated in this browser and never exported. To trust it, pin it in the daemon’s signers.toml and in every verifier’s registry:</div>');
      h.push('<div class="v mono">ta signers add --id ' + esc(d.signerId) + ' --method software --device "' + esc(d.name) + ' (web)" --pubkey ' + esc(d.pubkey) + '</div>');
      h.push('<div class="row" style="display:flex;gap:8px;margin-top:6px">' + btn('copy-cmd', 'copy that command', '', false) + btn('copy-key', 'copy key', 'ghost', false) + '</div>');
      h.push('<div class="hint">Device <code>' + esc(d.name) + '</code> · id <code>' + esc(d.id) + '</code>' + (S.registered ? ' · registered with the daemon' : '') + '</div>');
      h.push('<div class="row" style="display:flex;gap:8px;margin-top:6px">' + btn('register', 'register with daemon', 'ghost', S.busy) + btn('recheck', 'recheck pin', 'ghost', S.busy) + '</div>');
    }
    h.push('</div>');
    h.push('<div class="kv"><div class="k">Daemon</div><div class="v mono">' + esc(window.location.origin + (BASE || '')) + '</div><div class="hint">connection ' + esc(S.conn) + '</div></div>');
    h.push('</div>');
    return h.join('');
  }
  function renderGrant() {
    var g = S.grant;
    if (!g) return '';
    return '<div class="grant ' + (g.decision === 'deny' ? 'deny' : '') + '" id="p-grant"><div class="sub">' + (g.decision === 'deny' ? 'denied' : 'approved') + (g.answer ? ': ' + esc(g.answer) : '') + '</div>'
      + '<div class="gid">' + esc(g.id) + '</div><div class="sub dim">expires in ' + esc(fmtAge(Date.now(), Date.parse(g.expires_at), '')) + '</div>'
      + '<div class="qq">' + esc(g.question) + '</div>' + btn('grant-ok', 'done', 'yes') + '<div class="hint">the agent verifies this id with ta verify</div></div>';
  }
  function render() {
    var n = S.view === 'ask' ? current() : null;
    var keep = {};
    ['p-note', 'p-text'].forEach(function (id) { var el = $(id); if (el) keep[id] = { v: el.value, f: document.activeElement === el, s: el.selectionStart }; });
    var detailScroll = $('p-detail') ? $('p-detail').scrollTop : 0;
    root.innerHTML = (S.view === 'settings' ? renderSettings() : n ? renderAsk(n) : renderList()) + renderGrant();
    Object.keys(keep).forEach(function (id) {
      var el = $(id); if (!el) return;
      el.value = keep[id].v;
      if (keep[id].f) { el.focus(); try { el.setSelectionRange(keep[id].s, keep[id].s); } catch (_) {} }
    });
    if (detailScroll && $('p-detail')) $('p-detail').scrollTop = detailScroll;
  }
  function tick() {
    var now = Date.now(), before = S.queue.length;
    normalize();
    if (S.queue.length !== before) return render();
    if (S.msgCls === 'ok' && S.msg && now - S.msgAt > OK_MSG_MS) { setMsg(''); return render(); }
    var ages = root.querySelectorAll('.age[data-ts]');
    for (var i = 0; i < ages.length; i++) ages[i].textContent = fmtAge(ages[i].getAttribute('data-ts'), now);
  }

  // ── input ──────────────────────────────────────────────────────────────
  root.addEventListener('click', function (e) {
    var t = e.target;
    var nav = t.closest && t.closest('[data-nav]');
    if (nav) { var v = nav.getAttribute('data-nav'); if (v === 'list') return back(); S.view = v; setMsg(''); render(); if (v === 'settings') { checkEnrolment(); fetchVapid().then(currentSubscription).then(render); } return; }
    var card = t.closest && t.closest('[data-open]');
    if (card) return open(card.getAttribute('data-open'));
    var b = t.closest && t.closest('[data-act]');
    if (!b || b.disabled) return;
    var a = b.getAttribute('data-act'), arg = b.getAttribute('data-arg');
    if (a === 'grant-ok') { S.grant = null; return render(); }
    if (a === 'closed-ok') { S.closed = null; return render(); }
    if (a === 'enable-push') return enablePush();
    if (a === 'test-push') return testPush();
    if (a === 'register') return run(register(null), 'registered');
    if (a === 'recheck') return checkEnrolment();
    if (a === 'copy-key') return copy(S.dev.pubkey);
    if (a === 'copy-cmd') return copy('ta signers add --id ' + S.dev.signerId + ' --method software --device "' + S.dev.name + ' (web)" --pubkey ' + S.dev.pubkey);
    if (a === 'suggest') { var el = $('p-text'); if (el) { el.value = arg; } return; }
    if (a === 'once') return act('once');
    act(a, arg == null ? undefined : (a === 'pick' || a === 'ttl' ? Number(arg) : arg));
  });
  root.addEventListener('change', function (e) {
    if (e.target && e.target.getAttribute && e.target.getAttribute('data-act') === 'once') act('once');
  });
  root.addEventListener('keydown', function (e) {
    if (e.key === 'Enter' && e.target && e.target.id === 'p-text' && e.target.tagName === 'INPUT') { e.preventDefault(); act('confirm'); }
  });
  document.addEventListener('visibilitychange', function () {
    if (document.visibilityState === 'visible') { load(); if (!es) connect(); }
  });
  window.addEventListener('hashchange', function () { var id = decodeURIComponent(location.hash.slice(1)); if (id) open(id); });
  if (navigator.serviceWorker) navigator.serviceWorker.addEventListener('message', onSwMessage);

  // ── boot ───────────────────────────────────────────────────────────────
  window.SjbisPhone = { state: S, act: act, open: open, back: back, load: load, render: render, md: md, approve: approve, register: register, checkEnrolment: checkEnrolment };
  render();
  setInterval(tick, 1000);
  setInterval(load, 60000); // snoozed asks come back once their snooze lapses
  pushState();
  loadDevice().then(function () {
    render();
    return Promise.all([checkEnrolment(), fetchVapid(), swRegister().then(currentSubscription)]);
  }).then(function () {
    render();
    // Keep the daemon's device row current (name, key, and the push subscription if any).
    return currentSubscription().then(function (sub) { return register(sub ? sub.toJSON() : null); }).catch(function () {});
  }).catch(function (e) { setMsg('key setup failed: ' + (e.message || e), 'bad'); })
    .then(function () {
      return load().then(function () {
        connect();
        var id = decodeURIComponent(location.hash.slice(1));
        if (id) open(id);
      });
    });
}());
