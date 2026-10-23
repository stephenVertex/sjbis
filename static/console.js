// SJBIS console: one ask at a time, keyboard/Stream Deck driven, no framework.
// Plain asks POST /answer/{id}; approval asks go to the local signing sidecar
// (`ta console`), which posts the signed grant to the daemon itself.
(function () {
  'use strict';

  var BASE = window.__SJBIS_BASE_PATH__ != null ? window.__SJBIS_BASE_PATH__
    : window.location.pathname.replace(/\/console(\.html)?\/?$/, '').replace(/\/+$/, '');
  var SIDECAR = (new URLSearchParams(window.location.search).get('sidecar') || 'http://127.0.0.1:7879').replace(/\/+$/, '');
  var PRESET_TTLS = [[600, '10m'], [3600, '1h'], [86400, '24h']];
  function fmtTtl(secs) {
    if (secs % 86400 === 0) return (secs / 86400) + 'd';
    if (secs % 3600 === 0) return (secs / 3600) + 'h';
    if (secs % 60 === 0) return (secs / 60) + 'm';
    return secs + 's';
  }
  // TTL choices for an ask: the agent's requested TTL first (whatever it is),
  // then the presets. T cycles through them; the default is the requested one.
  function ttls(n) {
    var t = n && n.ttl_secs;
    if (!t || PRESET_TTLS.some(function (p) { return p[0] === t; })) return PRESET_TTLS;
    return [[t, fmtTtl(t) + ' (asked)']].concat(PRESET_TTLS);
  }
  var SNOOZE_MINUTES = 15;
  var OK_MSG_MS = 4000; // "answered: …" confirmations fade after this, so they never linger on the next ask
  var DISMISS_ARM_MS = 2500; // second DISMISS press must land within this window
  var GRANT_SHOW_MS = 5000;
  var CHOICE_TYPES = { multichoice: 1, picklist: 1, schedule: 1, approval: 1 };

  var S = {
    queue: [], cur: null, sel: null, ttlIdx: 1, busy: false, msg: '', msgCls: '',
    conn: 'connecting', sidecarUp: false, signer: '', sidecarWhy: 'checking…', grant: null,
  };
  var root = document.getElementById('console');
  if (!root) { root = document.createElement('div'); root.id = 'console'; document.body.appendChild(root); }
  root.innerHTML = '<div class="hdr" id="c-hdr"></div><div class="card" id="c-card"></div>'
    + '<div class="noterow" id="c-noterow" hidden><input id="c-note" autocomplete="off" spellcheck="false"></div>'
    + '<div class="row tools" id="c-tools"></div><div id="c-overlay"></div>';
  var $ = function (id) { return document.getElementById(id); };
  var noteEl = $('c-note');

  // ── helpers ────────────────────────────────────────────────────────────
  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }
  function inline(s) {
    return esc(s).replace(/`([^`]+)`/g, '<code>$1</code>').replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  }
  // Minimal, safe markdown: headings, paragraphs, bullet lists, fenced code, `code`, **bold**.
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
  function clock() { return new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }); }
  function current() { for (var i = 0; i < S.queue.length; i++) if (S.queue[i].id === S.cur) return S.queue[i]; return null; }
  function isApproval(n) { return n.question_type === 'approval'; }
  function isNumeric(n) { return n && n.question_type === 'numeric'; }
  function numStart(n) {
    if (!isNumeric(n)) return null;
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
    return lo || hi ? lo + '\u2013' + hi : 'any number';
  }
  function numAnswer(n) {
    var typed = note(), v = typed ? Number(typed.replace(/,/g, '')) : S.num;
    if (typed && !isFinite(v)) { setMsg('"' + typed + '" is not a number', 'bad'); return null; }
    if ((typeof n.min === 'number' && v < n.min) || (typeof n.max === 'number' && v > n.max)) {
      setMsg(numFmt(v) + ' is outside ' + numRange(n), 'bad'); return null;
    }
    return numFmt(v) + (n.unit ? ' ' + n.unit : '');
  }
  function options(n) {
    if (n.choices && n.choices.length) {
      return n.choices.map(function (c) { return typeof c === 'string' ? { value: c, label: c } : { value: c.value, label: c.label || c.value }; });
    }
    if (n.question_type === 'picklist' && n.items) return n.items.map(function (it) { return { value: it.title, label: it.title }; });
    if (n.question_type === 'schedule' && n.slots) {
      return n.slots.map(function (s) { var v = s.day + ' · ' + s.time; return { value: v, label: v }; });
    }
    return [];
  }
  function setMsg(text, cls) { S.msg = text || ''; S.msgCls = cls || ''; }
  function disarmDismiss(rerender) {
    S.dismissArm = null; S.dismissArmUntil = 0;
    if (S.msg === 'press DISMISS again to confirm') setMsg('');
    if (rerender) render();
  }

  // ── queue ──────────────────────────────────────────────────────────────
  function live(n, now) {
    return n && (!n.status || n.status === 'open')
      && !(n.snooze_until && Date.parse(n.snooze_until) > now)
      && !(n.deadline && Date.parse(n.deadline) <= now);
  }
  function selectAsk(id) {
    if (id === S.cur) return;
    S.cur = id; S.sel = null; setMsg('');
    S.dismissArm = null; S.dismissArmUntil = 0;
    S.num = numStart(current());
    noteEl.value = '';
    if (document.activeElement === noteEl) noteEl.blur(); // focus never carries over to the next ask
    var n = current(), t = n && n.ttl_secs, list = ttls(n);
    S.ttlIdx = 1;
    list.forEach(function (p, i) { if (p[0] === t) S.ttlIdx = i; });
  }
  function normalize() {
    var now = Date.now(), at = -1;
    S.queue.forEach(function (n, i) { if (n.id === S.cur) at = i; });
    var keep = S.queue.filter(function (n) { return live(n, now); });
    keep.sort(function (a, b) { return Date.parse(a.created_at) - Date.parse(b.created_at); });
    S.queue = keep;
    if (!current()) selectAsk(keep.length ? keep[Math.max(0, Math.min(at, keep.length - 1))].id : null);
  }
  function upsert(n) {
    S.queue = S.queue.filter(function (q) { return q.id !== n.id; }).concat([n]);
    normalize(); render();
  }
  function removeAsk(id) {
    var i = -1;
    S.queue.forEach(function (n, j) { if (n.id === id) i = j; });
    if (i < 0) return;
    S.queue.splice(i, 1);
    if (S.cur === id) { S.cur = null; selectAsk(S.queue.length ? S.queue[Math.min(i, S.queue.length - 1)].id : null); }
    render();
  }
  function step(d) {
    if (!S.queue.length) return;
    var i = 0;
    S.queue.forEach(function (n, j) { if (n.id === S.cur) i = j; });
    selectAsk(S.queue[(i + d + S.queue.length) % S.queue.length].id);
    render();
  }

  // ── network ────────────────────────────────────────────────────────────
  function readJson(r) {
    return r.json().catch(function () { return {}; }).then(function (j) {
      if (!r.ok) throw new Error((j.error || 'HTTP ' + r.status) + (j.code ? ' [' + j.code + ']' : ''));
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
      else if (ev.event === 'notification_answered') removeAsk(ev.envelope && ev.envelope.id);
      else if (ev.event === 'notification_cancelled' || ev.event === 'notification_dismissed') removeAsk(ev.id);
    };
    es.onerror = function () {
      if (es) es.close();
      es = null; everOpen = true; S.conn = 'reconnecting'; render();
      setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, 30000);
    };
  }
  function checkSidecar() {
    var ctl = window.AbortController ? new AbortController() : null;
    var timer = ctl && setTimeout(function () { ctl.abort(); }, 4000);
    return fetch(SIDECAR + '/health', { cache: 'no-store', signal: ctl && ctl.signal }).then(readJson).then(function (j) {
      if (j.ok !== true) throw new Error('health not ok');
      S.sidecarUp = true; S.signer = j.signer || '?'; S.sidecarWhy = '';
    }).catch(function (e) {
      S.sidecarUp = false; S.sidecarWhy = (e && e.name === 'AbortError') ? 'no reply from ' + SIDECAR : (e.message || String(e));
    }).then(function () { clearTimeout(timer); render(); });
  }

  // ── actions ────────────────────────────────────────────────────────────
  function run(promise, okText) {
    S.busy = true; setMsg('working…'); render();
    return promise.then(function () { setMsg(okText || '', 'ok'); S.msgAt = Date.now(); })
      .catch(function (e) { setMsg(e.message || String(e), 'bad'); })
      .then(function () { S.busy = false; render(); });
  }
  function note() { var v = noteEl.value.trim(); return v || null; }
  function answer(n, value) {
    var body = { answer: value, via: 'console', renderer: 'console' };
    if (note()) body.note = note();
    return run(post(BASE + '/answer/' + encodeURIComponent(n.id), body).then(function () { removeAsk(n.id); }), 'answered: ' + value);
  }
  function approve(n, decision, value) {
    if (!S.sidecarUp) { setMsg('signing sidecar offline: ' + S.sidecarWhy, 'bad'); render(); return; }
    var body = { ask_id: n.id, decision: decision, answer: value == null ? null : value, note: note(), ttl_secs: ttls(n)[S.ttlIdx][0], once: n.once === true };
    return run(post(SIDECAR + '/approve', body).then(function (g) {
      S.grant = { id: g.id, expires_at: g.expires_at, decision: g.decision || decision };
      removeAsk(n.id);
      clearTimeout(S.grantTimer);
      S.grantTimer = setTimeout(closeGrant, GRANT_SHOW_MS);
    }).catch(function (e) {
      if (e instanceof TypeError) checkSidecar();
      throw e;
    }));
  }
  function closeGrant() { clearTimeout(S.grantTimer); S.grant = null; render(); }
  function act(name, arg) {
    var n = current();
    if (name !== 'dismiss' && S.dismissArm != null) disarmDismiss(true); // any other action cancels a pending dismiss
    if (name === 'prev') return step(-1);
    if (name === 'next') return step(1);
    if (name === 'reload') { setMsg(''); checkSidecar(); if (!es) connect(); return load(); }
    if (!n || S.busy) return;
    var t = n.question_type, opts = options(n);
    if (name === 'note') { noteEl.focus(); return; }
    if (isApproval(n) && !S.sidecarUp && /^(pick|yes|no|confirm|ttl)$/.test(name)) {
      setMsg('signing sidecar offline: ' + S.sidecarWhy, 'bad'); return render();
    }
    if (name === 'pick') { if (arg < opts.length) { S.sel = arg; setMsg(''); render(); } return; }
    if (name === 'ttl') { if (isApproval(n)) { S.ttlIdx = (S.ttlIdx + 1) % ttls(n).length; render(); } return; }
    if (name === 'snooze') {
      var mins = SNOOZE_MINUTES;
      if (n.deadline) mins = Math.min(mins, Math.floor((Date.parse(n.deadline) - Date.now()) / 60000));
      if (mins < 1) { setMsg('deadline too close to snooze', 'bad'); return render(); }
      return run(post(BASE + '/snooze/' + encodeURIComponent(n.id), { minutes: mins }).then(function () { removeAsk(n.id); }), 'snoozed ' + mins + 'm');
    }
    if (name === 'dismiss') {
      // Two presses within DISMISS_ARM_MS: a stray tap must not throw an ask away.
      if (!(S.dismissArm === n.id && Date.now() < S.dismissArmUntil)) {
        S.dismissArm = n.id; S.dismissArmUntil = Date.now() + DISMISS_ARM_MS;
        setMsg('press DISMISS again to confirm', 'warn');
        setTimeout(function () { if (S.dismissArm === n.id && Date.now() >= S.dismissArmUntil) disarmDismiss(true); }, DISMISS_ARM_MS + 50);
        return render();
      }
      disarmDismiss(false);
      return run(post(BASE + '/dismiss/' + encodeURIComponent(n.id)).then(function () { removeAsk(n.id); }), 'dismissed');
    }
    if (isApproval(n)) {
      if (name === 'no') return approve(n, 'deny', null);
      if (name === 'confirm' && (!opts.length || S.sel == null)) return;
      if (name === 'yes' || name === 'confirm') {
        if (opts.length && S.sel == null) { setMsg('pick 1-' + Math.min(opts.length, 9) + ' first', 'bad'); return render(); }
        return approve(n, 'approve', opts.length ? opts[S.sel].value : null);
      }
      return;
    }
    if (isNumeric(n)) {
      // YES/NO (and +/-) step the value; Enter sends it, or sends what was typed in the note field.
      if (name === 'yes' || name === 'no') { S.num = numClamp(n, S.num + (name === 'yes' ? 1 : -1) * numStep(n)); setMsg(''); return render(); }
      if (name === 'confirm') {
        var nv = numAnswer(n);
        if (nv == null) return render();
        noteEl.value = '';
        return answer(n, nv);
      }
      return;
    }
    if (name === 'yes') {
      if (t === 'yesno') return answer(n, n.yes_label || 'Yes');
      if (t === 'diff') return answer(n, 'Approved');
      if (t === 'ack') return answer(n, n.ack_label || 'Acknowledged');
    }
    if (name === 'no') {
      if (t === 'yesno') return answer(n, n.no_label || 'No');
      if (t === 'diff') return answer(n, 'Rejected');
    }
    if (name === 'confirm') {
      if (S.sel != null && opts[S.sel]) return answer(n, opts[S.sel].value);
      if (t === 'freetext' && note()) { var text = note(); noteEl.value = ''; return answer(n, text); }
      if (t === 'ack') return answer(n, n.ack_label || 'Acknowledged');
    }
  }

  // ── render ─────────────────────────────────────────────────────────────
  function btn(act, key, label, cls, disabled, arg) {
    return '<button type="button" data-act="' + act + '"' + (arg != null ? ' data-arg="' + arg + '"' : '')
      + ' class="' + (cls || '') + '"' + (disabled ? ' disabled' : '') + '><kbd>' + esc(key) + '</kbd><span class="lbl">'
      + esc(label) + '</span></button>';
  }
  function renderHeader(n) {
    var i = 0;
    S.queue.forEach(function (q, j) { if (q.id === S.cur) i = j; });
    var conn = S.conn === 'live' ? '<span class="dot ok">live</span>' : '<span class="dot warn">' + esc(S.conn) + '</span>';
    var side = S.sidecarUp ? '<span class="ok">signer: ' + esc(S.signer) + '</span>' : '<span class="bad">signing sidecar offline</span>';
    $('c-hdr').innerHTML = (n ? '<span class="pos" id="c-pos">' + (i + 1) + ' of ' + S.queue.length + '</span>' : '<span class="pos">SJBIS</span>')
      + '<span class="sp"></span>' + side + conn + '<span id="c-clock">' + esc(clock()) + '</span>';
  }
  function renderCard(n) {
    var card = $('c-card');
    if (!n) {
      card.innerHTML = '<div class="empty"><div class="clock" id="c-bigclock">' + esc(clock()) + '</div>'
        + '<div class="calm">no open asks</div><div class="calm ' + (S.conn === 'live' ? 'ok' : 'warn') + '">daemon ' + esc(S.conn) + '</div></div>';
      card.dataset.id = '';
      return;
    }
    var keepScroll = card.dataset.id === n.id && $('c-detail') ? $('c-detail').scrollTop : 0;
    var appr = isApproval(n), opts = options(n), t = n.question_type, down = appr && !S.sidecarUp;
    var dis = S.busy || down, h = [];
    h.push('<div class="meta"><b>' + esc(n.agent_name) + '</b> · ' + esc(n.src) + ' · <span class="urg' + (n.urgency >= 4 ? ' hot' : '')
      + '">U' + esc(n.urgency) + '</span> · <span id="c-age">' + esc(fmtAge(n.created_at, Date.now())) + '</span></div>');
    h.push('<h1 class="q' + (n.question.length > 110 ? ' long' : '') + '">' + esc(n.question) + '</h1>');
    if (n.detail_markdown) h.push('<div class="detail" id="c-detail">' + md(n.detail_markdown) + '</div>');
    else if (n.detail) h.push('<div class="detail plain" id="c-detail">' + esc(n.detail) + '</div>');
    else h.push('<div class="spacer"></div>');
    if (appr) {
      h.push(down ? '<div class="lock bad">🔒 signing sidecar offline: approvals disabled (' + esc(S.sidecarWhy) + ')</div>'
        : '<div class="lock warn">🔒 signed approval · signer ' + esc(S.signer) + (n.once ? ' · single-use' : '') + '</div>');
    }
    var row = [];
    opts.slice(0, 9).forEach(function (o, i) {
      row.push(btn('pick', String(i + 1), o.label, S.sel === i ? 'sel' : '', dis, i));
    });
    if (appr) {
      if (row.length) { h.push('<div class="row">' + row.join('') + '</div>'); row = []; }
      row.push(btn('yes', 'Y', opts.length ? 'approve choice' : 'approve', 'yes', dis || (opts.length && S.sel == null)));
      row.push(btn('no', 'N', 'deny', 'no', dis));
      row.push(btn('ttl', 'T', 'ttl ' + ttls(n)[S.ttlIdx][1], '', dis));
    } else if (t === 'yesno' || t === 'diff') {
      row.push(btn('yes', 'Y', t === 'diff' ? 'Approve' : n.yes_label || 'Yes', 'yes', S.busy));
      row.push(btn('no', 'N', t === 'diff' ? 'Reject' : n.no_label || 'No', 'no', S.busy));
    } else if (t === 'ack') {
      row.push(btn('yes', 'Y', n.ack_label || 'Acknowledged', 'yes', S.busy));
    } else if (t === 'numeric') {
      h.push('<div class="num" id="c-num">' + esc(numFmt(S.num)) + (n.unit ? ' <span class="unit">' + esc(n.unit) + '</span>' : '')
        + '<span class="range">' + esc(numRange(n)) + ' · step ' + esc(numFmt(numStep(n))) + '</span></div>');
      row.push(btn('no', 'N', '\u2212' + numFmt(numStep(n)), 'no', S.busy));
      row.push(btn('yes', 'Y', '+' + numFmt(numStep(n)), 'yes', S.busy));
      row.push(btn('confirm', '\u23ce', 'send (M to type a number)', 'yes', S.busy));
    } else if (t === 'freetext') {
      row.push(btn('confirm', '⏎', 'send text (M to type)', 'yes', S.busy));
    } else if (!opts.length) {
      row.push('<div class="lock warn">open this ask on the dashboard to answer it</div>');
    }
    if (opts.length && !appr) row.push(btn('confirm', '⏎', 'confirm', 'yes', S.busy || S.sel == null));
    h.push('<div class="row">' + row.join('') + '</div>');
    h.push('<div class="msg ' + S.msgCls + '">' + esc(S.msg) + '</div>');
    card.innerHTML = h.join('');
    card.dataset.id = n.id;
    if (keepScroll && $('c-detail')) $('c-detail').scrollTop = keepScroll;
  }
  function render() {
    var n = current();
    renderHeader(n);
    renderCard(n);
    $('c-noterow').hidden = !n;
    noteEl.placeholder = n && n.question_type === 'freetext' ? 'M: type the answer, Enter sends, Esc leaves'
      : isNumeric(n) ? 'M: type a number (' + numRange(n) + '), Enter sends, Esc leaves' : 'M: optional note, Esc leaves';
    $('c-tools').innerHTML = [btn('prev', '←/K', 'prev'), btn('next', '→/J', 'next'), btn('note', 'M', 'note', '', !n),
      btn('snooze', 'S', 'snooze ' + SNOOZE_MINUTES + 'm', '', !n || S.busy), btn('dismiss', 'D', n && S.dismissArm === n.id ? 'dismiss?' : 'dismiss', n && S.dismissArm === n.id ? 'arm' : '', !n || S.busy),
      btn('reload', 'R', 'reload')].join('');
    var g = S.grant;
    $('c-overlay').innerHTML = !g ? '' : '<div class="grant ' + (g.decision === 'deny' ? 'deny' : '') + '" id="c-grant">'
      + '<div class="sub">' + (g.decision === 'deny' ? 'denied' : 'approved') + '</div><div class="gid">' + esc(g.id) + '</div>'
      + '<div class="sub">expires in ' + esc(fmtAge(Date.now(), Date.parse(g.expires_at), '')) + '</div>'
      + '<div class="hint">press any key to continue</div></div>';
  }
  function tick() {
    var now = Date.now(), before = S.queue.length;
    normalize();
    if (S.queue.length !== before) return render();
    var n = current();
    if (S.msgCls === 'ok' && S.msg && now - (S.msgAt || 0) > OK_MSG_MS) { setMsg(''); return render(); }
    if ($('c-clock')) $('c-clock').textContent = clock();
    if ($('c-bigclock')) $('c-bigclock').textContent = clock();
    if (n && $('c-age')) $('c-age').textContent = fmtAge(n.created_at, now);
  }

  // ── input ──────────────────────────────────────────────────────────────
  var KEYS = { y: 'yes', n: 'no', enter: 'confirm', t: 'ttl', m: 'note', s: 'snooze', d: 'dismiss',
    arrowleft: 'prev', k: 'prev', arrowright: 'next', j: 'next', r: 'reload' };
  document.addEventListener('keydown', function (e) {
    // Ctrl+Alt+Shift+<key> is a "hard" key (the Stream Deck sends these): it
    // acts even while the note field has focus. The desktop owns plain
    // Ctrl+Alt combos (terminal, workspaces), so all three are required.
    // Any other modifier combination is ignored.
    var hard = e.ctrlKey && e.altKey && e.shiftKey && !e.metaKey;
    if ((e.ctrlKey || e.metaKey || e.altKey) && !hard) return;
    var key = (e.key || '').toLowerCase();
    if (hard) { // Shift changes e.key ("1" -> "!"), so go by the physical key
      var c = e.code || '';
      if (/^Digit[0-9]$/.test(c)) key = c.slice(5);
      else if (/^Key[A-Z]$/.test(c)) key = c.slice(3).toLowerCase();
    }
    if (S.grant) { e.preventDefault(); closeGrant(); return; }
    if (hard && e.target === noteEl) noteEl.blur();
    if (e.target === noteEl) {
      if (key === 'escape') { noteEl.blur(); e.preventDefault(); }
      if (key === 'enter') {
        e.preventDefault();
        var n = current();
        if (n && (n.question_type === 'freetext' || isNumeric(n))) act('confirm'); else noteEl.blur();
      }
      return;
    }
    var d = $('c-detail');
    if (d && (key === 'pagedown' || key === 'pageup' || key === 'arrowdown' || key === 'arrowup')) {
      d.scrollTop += (key === 'pagedown' || key === 'arrowdown' ? 1 : -1) * d.clientHeight * 0.8;
      e.preventDefault(); return;
    }
    if (/^[1-9]$/.test(key)) { e.preventDefault(); act('pick', Number(key) - 1); return; }
    if (isNumeric(current()) && /^[+=\-_]$/.test(key)) { e.preventDefault(); act(key === '+' || key === '=' ? 'yes' : 'no'); return; }
    if (KEYS[key]) { e.preventDefault(); act(KEYS[key]); }
  });
  root.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('button[data-act]');
    if (!b || b.disabled) return;
    if (S.grant) return closeGrant();
    var a = b.getAttribute('data-act'), arg = b.getAttribute('data-arg');
    if (a === 'pick' && Number(arg) === S.sel) return act('confirm');
    act(a, arg == null ? undefined : Number(arg));
  });
  $('c-overlay').addEventListener('click', function () { if (S.grant) closeGrant(); });

  window.SjbisConsole = { state: S, act: act, load: load, checkSidecar: checkSidecar, render: render, md: md, sidecar: SIDECAR };
  render();
  checkSidecar();
  setInterval(checkSidecar, 10000);
  setInterval(tick, 1000);
  setInterval(load, 60000); // snoozed asks reappear once their snooze lapses
  load().then(connect);
}());
