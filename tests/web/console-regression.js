(function () {
  'use strict';

  window.__SJBIS_BASE_PATH__ = '';
  const SIDECAR = 'http://127.0.0.1:7879';
  const MARKDOWN = [
    '## Release notes',
    '',
    'Ship **v2** now? Run `make deploy`.',
    '',
    '- staging green',
    '- prod idle',
    '',
    '<script>window.__xss = 1</script><img src=x onerror="window.__xss = 2">',
  ].join('\n');
  const ask = (id, minute, extra) => Object.assign({
    id, agent_name: 'fixture-bot', sender: 'fixture-bot', src: 'fixture-bot · console', urgency: 2,
    blocking: false, status: 'open', created_at: `2026-10-23T10:0${minute}:00Z`,
  }, extra);
  const asks = [
    ask('c-yesno', 0, { question: 'Ship the release?', question_type: 'yesno', yes_label: 'Ship it', no_label: 'Hold', detail_markdown: MARKDOWN }),
    ask('c-choice', 1, { question: 'Which lunch?', question_type: 'multichoice', detail: 'Plain <b>detail</b>',
      choices: [{ value: 'thai', label: 'Thai' }, { value: 'indian', label: 'Indian' }, { value: 'salad', label: 'Salad' }] }),
    ask('c-appr-choice', 2, { question: 'Deploy which plan?', question_type: 'approval', ttl_secs: 3600, urgency: 4,
      choices: [{ value: 'plan A', label: 'plan A' }, { value: 'plan B', label: 'plan B' }] }),
    ask('c-appr', 3, { question: 'Rotate the key?', question_type: 'approval', ttl_secs: 600 }),
    ask('c-ack', 4, { question: 'Backup finished', question_type: 'ack' }),
  ];

  const h = { answers: [], approvals: [], snoozes: [], dismisses: [], passes: [], uncaught: [],
    healthUp: false, approveFail: false, sources: [] };
  window.__consoleRegression = h;
  window.addEventListener('error', (e) => h.uncaught.push(e.error || e.message));
  window.addEventListener('unhandledrejection', (e) => h.uncaught.push(e.reason));

  const json = (body, status) => new Response(JSON.stringify(body), { status: status || 200, headers: { 'Content-Type': 'application/json' } });
  window.fetch = async (input, init) => {
    const url = String(input && input.url ? input.url : input);
    const method = (init && init.method) || 'GET';
    const body = init && init.body ? JSON.parse(init.body) : null;
    if (url === `${SIDECAR}/health`) {
      if (!h.healthUp) throw new TypeError('Failed to fetch');
      return json({ ok: true, signer: 'pi-yubikey', daemon: 'http://daemon.test:7878' });
    }
    if (url === `${SIDECAR}/approve` && method === 'POST') {
      h.approvals.push({ body, headers: init.headers });
      if (h.approveFail) return json({ error: 'signer locked', code: 'yubikey_absent' }, 503);
      const exp = new Date(Date.now() + body.ttl_secs * 1000).toISOString().replace(/\.\d+Z$/, 'Z');
      return json({ id: body.decision === 'deny' ? 'ta-deny01' : 'ta-abc123', expires_at: exp, decision: body.decision });
    }
    if (url.startsWith('/list')) return json(asks);
    let m = url.match(/^\/answer\/([^/?]+)$/);
    if (m && method === 'POST') { h.answers.push({ id: decodeURIComponent(m[1]), body }); return json({ id: m[1] }); }
    m = url.match(/^\/snooze\/([^/?]+)$/);
    if (m && method === 'POST') { h.snoozes.push({ id: decodeURIComponent(m[1]), body }); return json({ id: m[1], status: 'open' }); }
    m = url.match(/^\/dismiss\/([^/?]+)$/);
    if (m && method === 'POST') { h.dismisses.push(decodeURIComponent(m[1])); return json({ id: m[1] }); }
    return json({ error: `unexpected ${method} ${url}` }, 500);
  };
  window.EventSource = class FixtureEventSource {
    constructor(url) { this.url = url; h.sources.push(this); setTimeout(() => this.onopen && this.onopen(new Event('open')), 0); }
    emit(ev) { this.onmessage({ data: JSON.stringify(ev) }); }
    close() { this.closed = true; }
  };

  const $ = (sel) => document.querySelector(sel);
  const text = (sel) => ($(sel) ? $(sel).textContent : '');
  const key = (k, target) => (target || document).dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const button = (act) => $(`button[data-act="${act}"]`);
  function record(message, cls) {
    const li = document.createElement('li'); li.className = cls; li.textContent = message;
    document.getElementById('regression-results').appendChild(li);
  }
  function assert(cond, message) { if (!cond) throw new Error(message); h.passes.push(message); record(message, 'pass'); }
  function waitFor(fn, message, ms) {
    const end = Date.now() + (ms || 5000);
    return new Promise((resolve, reject) => {
      (function poll() {
        const v = fn();
        if (v) resolve(v); else if (Date.now() > end) reject(new Error(`Timed out: ${message}`)); else setTimeout(poll, 20);
      }());
    });
  }
  const question = () => text('#c-card .q');

  async function run() {
    await waitFor(() => question() === 'Ship the release?', 'oldest ask renders first');
    assert(text('#c-pos') === '1 of 5', 'queue position shows 1 of 5');
    assert(text('#c-card .meta').includes('fixture-bot · console') && text('#c-card .meta').includes('U2'), 'meta shows source and urgency');
    assert($('#c-detail h3') && $('#c-detail strong') && $('#c-detail code') && $('#c-detail li'), 'markdown renders heading, bold, code, list');
    assert(!$('#c-detail script') && !$('#c-detail img') && text('#c-detail').includes('<script>'), 'markdown escapes raw HTML');
    assert(text('button[data-act="yes"]').includes('Ship it') && text('button[data-act="no"]').includes('Hold'), 'yes/no labels shown with key hints');
    key('ArrowRight'); assert(question() === 'Which lunch?', 'Right moves to the next ask');
    key('k'); assert(question() === 'Ship the release?', 'K moves to the previous ask');
    key('j'); key('ArrowLeft'); assert(question() === 'Ship the release?', 'J/Left navigate');

    key('m');
    const note = $('#c-note');
    assert(document.activeElement === note, 'M focuses the note field');
    note.value = 'looks good';
    key('y', note); assert(h.answers.length === 0, 'typing in the note field does not trigger actions');
    key('Escape', note); assert(document.activeElement !== note, 'Esc leaves the note field');
    key('y');
    await waitFor(() => h.answers.length === 1, 'yes answer posted');
    const a0 = h.answers[0];
    assert(a0.id === 'c-yesno' && a0.body.answer === 'Ship it' && a0.body.via === 'console' && a0.body.renderer === 'console' && a0.body.note === 'looks good',
      'Y posts /answer with yes_label, via/renderer console and the note');
    await waitFor(() => question() === 'Which lunch?', 'advances to the next ask');
    assert(text('#c-pos') === '1 of 4', 'queue shrinks after answering');
    assert($('#c-detail.plain') && !$('#c-detail b'), 'plain detail is escaped');

    key('2');
    assert(button('pick') && $('button[data-arg="1"]').classList.contains('sel'), '2 selects the second choice');
    key('Enter');
    await waitFor(() => h.answers.length === 2, 'choice answer posted');
    assert(h.answers[1].id === 'c-choice' && h.answers[1].body.answer === 'indian' && !('note' in h.answers[1].body), 'Enter confirms the choice value');

    await waitFor(() => question() === 'Deploy which plan?', 'approval ask renders');
    assert(text('#c-hdr').includes('signing sidecar offline'), 'header shows the sidecar offline');
    assert(text('#c-card .lock').includes('signing sidecar offline'), 'approval card explains why it is disabled');
    assert(button('yes').disabled && button('no').disabled && button('pick').disabled, 'approval actions disabled while offline');
    assert(text('button[data-act="yes"]').includes('approve') && text('button[data-act="no"]').includes('deny'), 'approval shows approve/deny, not plain answers');
    key('1'); key('y'); key('n'); key('Enter');
    await sleep(50);
    assert(h.approvals.length === 0 && h.answers.length === 2, 'no requests while the sidecar is offline');

    h.healthUp = true;
    await window.SjbisConsole.checkSidecar();
    assert(text('#c-hdr').includes('signer: pi-yubikey'), 'header shows the signer when the sidecar is up');
    assert(text('button[data-act="ttl"]').includes('ttl 1h'), 'TTL defaults to 1h');
    key('t'); assert(text('button[data-act="ttl"]').includes('ttl 24h'), 'T cycles the TTL');
    key('y'); await sleep(30);
    assert(h.approvals.length === 0, 'approve with choices needs a picked choice');
    key('2'); key('Enter');
    await waitFor(() => $('#c-grant'), 'grant overlay shows');
    const ap = h.approvals[0].body;
    assert(ap.ask_id === 'c-appr-choice' && ap.decision === 'approve' && ap.answer === 'plan B' && ap.note === null
      && ap.ttl_secs === 86400 && ap.once === false, 'approve posts the contract body to the sidecar');
    assert(h.approvals[0].headers['Content-Type'] === 'application/json', 'approve sends JSON');
    assert(text('#c-grant .gid') === 'ta-abc123' && text('#c-grant').includes('expires in 23h'), 'grant id and expiry shown large');
    assert(h.answers.length === 2, 'approval never POSTs /answer');
    key('x');
    await waitFor(() => !$('#c-grant'), 'any key closes the grant overlay');
    assert(question() === 'Rotate the key?', 'advances after the grant');
    assert(text('button[data-act="ttl"]').includes('ttl 10m'), 'TTL defaults to the requested ttl_secs when it is a preset');

    h.approveFail = true;
    key('n');
    await waitFor(() => text('#c-card .msg').includes('signer locked [yubikey_absent]'), 'sidecar error is shown');
    assert(h.approvals[1].body.decision === 'deny' && h.approvals[1].body.answer === null, 'N denies');
    assert(question() === 'Rotate the key?', 'failed approval keeps the ask');
    h.approveFail = false;
    key('n');
    await waitFor(() => $('#c-grant.deny'), 'deny grant overlay shows');
    key(' ');
    await waitFor(() => question() === 'Backup finished', 'ack ask is next');

    const es = h.sources[h.sources.length - 1];
    es.emit({ event: 'notification_created', notification: ask('c-live', 5, { question: 'Live one?', question_type: 'yesno' }) });
    await waitFor(() => text('#c-pos') === '1 of 2', 'SSE created adds to the queue');
    es.emit({ event: 'notification_cancelled', id: 'c-live' });
    await waitFor(() => text('#c-pos') === '1 of 1', 'SSE cancelled removes from the queue');
    es.emit({ event: 'notification_created', notification: ask('c-live2', 6, { question: 'Dismiss me', question_type: 'yesno' }) });
    key('s');
    await waitFor(() => h.snoozes.length === 1, 'snooze posted');
    assert(h.snoozes[0].id === 'c-ack' && h.snoozes[0].body.minutes === 15, 'S snoozes 15 minutes');
    await waitFor(() => question() === 'Dismiss me', 'advances after snooze');
    key('d');
    await waitFor(() => text('#c-card .msg').includes('press DISMISS again'), 'first D only arms');
    assert(h.dismisses.length === 0, 'one D press does not dismiss');
    key('j');
    assert(!text('#c-card .msg').includes('press DISMISS again'), 'another key disarms');
    key('d'); key('d');
    await waitFor(() => h.dismisses.length === 1 && h.dismisses[0] === 'c-live2', 'D twice dismisses');
    await waitFor(() => text('#c-card .calm').includes('no open asks'), 'empty queue shows the calm screen');
    assert($('#c-bigclock'), 'empty screen shows the clock');
    es.emit({ event: 'notification_created', notification: ask('c-live3', 7, { question: 'Answered elsewhere', question_type: 'ack' }) });
    await waitFor(() => question() === 'Answered elsewhere', 'new ask appears live');
    es.emit({ event: 'notification_answered', envelope: { id: 'c-live3', renderer: 'ack', src: 'x', via: 'dashboard' } });
    await waitFor(() => !question(), 'SSE answered removes the ask');

    es.onerror(new Event('error'));
    assert(text('#c-hdr').includes('reconnecting'), 'SSE error shows reconnecting');
    await waitFor(() => h.sources.length >= 2 && text('#c-hdr').includes('live'), 'SSE reconnects', 4000);
    await waitFor(() => question() === 'Ship the release?' && text('#c-pos') === '1 of 5', 'reconnect re-GETs /list');

    assert(window.__xss === undefined, 'no injected script ran');
    assert(h.uncaught.length === 0, `no uncaught errors ${h.uncaught.join(';')}`);
    document.body.dataset.regression = 'passed';
    document.getElementById('regression-title').textContent = `Console regression: passed (${h.passes.length})`;
  }

  window.addEventListener('load', () => {
    run().catch((e) => {
      document.body.dataset.regression = 'failed';
      document.getElementById('regression-title').textContent = 'Console regression: failed';
      record(e.stack || e.message || String(e), 'fail');
    });
  });
}());
