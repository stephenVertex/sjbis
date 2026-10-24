(function () {
  'use strict';

  window.__SJBIS_BASE_PATH__ = '';
  const C = window.SjbisPhoneCrypto;
  const ask = (id, minute, extra) => Object.assign({
    id, agent_name: 'fixture-bot', sender: 'fixture-bot', src: 'fixture-bot · phone', urgency: 2,
    blocking: false, status: 'open', created_at: `2026-10-23T10:0${minute}:00Z`,
  }, extra);
  const LONG = 'Why is this so long? '.repeat(40);
  const asks = [
    ask('p-yesno', 0, { question: 'Ship the release?', question_type: 'yesno', yes_label: 'Ship it', no_label: 'Hold',
      detail_markdown: '## Notes\n\nRun `make deploy`.\n\n- staging green\n\n<script>window.__xss = 1</script><img src=x onerror="window.__xss = 2">' }),
    ask('p-choice', 1, { question: 'Which lunch?', question_type: 'multichoice',
      choices: [{ value: 'thai', label: 'Thai' }, { value: 'indian', label: 'Indian' }] }),
    ask('p-appr', 2, { question: LONG, question_type: 'approval', ttl_secs: 600, urgency: 4, renews: 'ta-old001' }),
    ask('p-appr-choice', 3, { question: 'Deploy which plan?', question_type: 'approval', ttl_secs: 3600,
      choices: [{ value: 'plan A', label: 'plan A' }, { value: 'plan B', label: 'plan B' }] }),
    ask('p-num', 4, { question: 'How many?', question_type: 'numeric', min: 1, max: 10, step: 1, default_value: 4, unit: 'asks' }),
    ask('p-text', 5, { question: 'Name it', question_type: 'freetext', suggestions: ['kiosk', 'phone'] }),
    ask('p-form', 6, { question: 'Quick form', question_type: 'form', sub_questions: [
      { key: 'ok', question: 'Fine?', shape: 'yesno' }, { key: 'count', question: 'How many?', shape: 'numeric', min: 1, max: 9, default: 3 }] }),
    ask('p-ack', 7, { question: 'Backup finished', question_type: 'ack' }),
  ];

  const h = { answers: [], grants: [], registers: [], snoozes: [], dismisses: [], passes: [], uncaught: [], sources: [], grantFailOnce: false };
  window.__phoneRegression = h;
  window.addEventListener('error', (e) => {
    h.uncaught.push(e.error || e.message);
    const t = document.getElementById('regression-title');
    if (t && /running$/.test(t.textContent)) t.textContent = 'Phone regression: script error: ' + (e.message || e.error) + ' @' + (e.filename || '') + ':' + (e.lineno || '');
  });
  window.addEventListener('unhandledrejection', (e) => h.uncaught.push(e.reason));

  const json = (body, status) => new Response(JSON.stringify(body), { status: status || 200, headers: { 'Content-Type': 'application/json' } });
  let pinned = null; // the pubkey the fixture daemon has "pinned" for phone@iphone
  window.fetch = async (input, init) => {
    const url = String(input && input.url ? input.url : input);
    const method = (init && init.method) || 'GET';
    const body = init && init.body ? JSON.parse(init.body) : null;
    if (url.startsWith('/list')) return json(asks);
    if (url.startsWith('/signers')) return json(pinned ? [{ id: 'phone@iphone', method: 'software', pubkey: pinned }] : []);
    if (url.startsWith('/push/vapid')) return json({ enabled: true, public_key: 'BPtestkey', subject: 'mailto:t@t' });
    if (url === '/push/register' && method === 'POST') { h.registers.push(body); pinned = body.pubkey; return json({ ok: true, device_id: body.device_id, enrolled: true, push: !!body.subscription }); }
    if (url === '/grants' && method === 'POST') {
      h.grants.push(body);
      if (h.grantFailOnce) { h.grantFailOnce = false; return json({ error: 'id collision for ' + body.id, code: 'id_collision' }, 409); }
      return json({ id: body.id, kind: 'grant', status: 'valid' }, 201);
    }
    let m = url.match(/^\/answer\/([^/?]+)$/);
    if (m && method === 'POST') { h.answers.push({ id: decodeURIComponent(m[1]), body }); return json({ id: m[1] }); }
    m = url.match(/^\/snooze\/([^/?]+)$/);
    if (m && method === 'POST') { h.snoozes.push({ id: decodeURIComponent(m[1]), body }); return json({ id: m[1], status: 'open' }); }
    m = url.match(/^\/dismiss\/([^/?]+)$/);
    if (m && method === 'POST') { h.dismisses.push(decodeURIComponent(m[1])); return json({ id: m[1] }); }
    m = url.match(/^\/notification\/([^/?]+)$/);
    if (m) return json({ id: decodeURIComponent(m[1]), question: 'Old one', status: 'answered', answer_label: 'Yes', question_type: 'yesno' });
    return json({ error: `unexpected ${method} ${url}` }, 500);
  };
  window.EventSource = class FixtureEventSource {
    constructor(url) { this.url = url; h.sources.push(this); setTimeout(() => this.onopen && this.onopen(new Event('open')), 0); }
    emit(ev) { this.onmessage({ data: JSON.stringify(ev) }); }
    close() { this.closed = true; }
  };
  // In-memory IndexedDB: headless Chrome's virtual time outruns real IDB I/O, and key
  // persistence is exercised on the device anyway. Mirrors the calls phone.js makes.
  (function fakeIdb() {
    const stores = {};
    const req = (result) => { const r = { result }; setTimeout(() => r.onsuccess && r.onsuccess({ target: r }), 0); return r; };
    const db = {
      createObjectStore(name) { stores[name] = stores[name] || new Map(); },
      transaction(name) {
        const tx = {};
        tx.objectStore = () => ({
          get: (k) => req(stores[name].get(k)),
          put: (v, k) => { stores[name].set(k, v); return req(k); },
        });
        setTimeout(() => tx.oncomplete && tx.oncomplete(), 1);
        return tx;
      },
    };
    Object.defineProperty(window, 'indexedDB', { configurable: true, value: { open() {
      const r = { result: db };
      setTimeout(() => { if (!stores.kv) { db.createObjectStore('kv'); r.onupgradeneeded && r.onupgradeneeded(); } r.onsuccess && r.onsuccess(); }, 0);
      return r;
    } } });
    h.idbStores = stores;
  }());
  // A fake service worker container: no real SW is served by the test server.
  const fakeReg = { pushManager: { getSubscription: async () => null }, getNotifications: async () => [] };
  try {
    Object.defineProperty(navigator, 'serviceWorker', { configurable: true, value: {
      register: async () => fakeReg, getRegistration: async () => fakeReg, ready: Promise.resolve(fakeReg), addEventListener() {},
    } });
  } catch (e) { h.swStubError = String(e); }

  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => Array.from(document.querySelectorAll(sel));
  const text = (sel) => ($(sel) ? $(sel).textContent : '');
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const button = (act) => $(`button[data-act="${act}"]`);
  const click = (el) => el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  function record(message, cls) {
    const li = document.createElement('li'); li.className = cls; li.textContent = message;
    document.getElementById('regression-results').appendChild(li);
  }
  function assert(cond, message) { if (!cond) throw new Error(message); h.passes.push(message); record(message, 'pass'); }
  function waitFor(fn, message, ms) {
    const end = Date.now() + (ms || 5000);
    return new Promise((resolve, reject) => {
      (function poll() {
        let v;
        try { v = fn(); } catch (e) { return reject(new Error(message + ': ' + e.message)); }
        document.getElementById('regression-title').textContent = 'Phone regression: running (' + h.passes.length + ' passed, waiting: ' + message + ', dev=' + !!(window.SjbisPhone && window.SjbisPhone.state.dev) + ', q=' + (window.SjbisPhone ? window.SjbisPhone.state.queue.length : -1) + ', t=' + (Date.now() % 100000) + ')';
        if (v) return resolve(v);
        if (Date.now() > end) return reject(new Error('timeout: ' + message));
        setTimeout(poll, 20);
      }());
    });
  }
  const P = () => window.SjbisPhone;
  const S = () => P().state;
  const listAgain = () => waitFor(() => S().view === 'list' && !S().busy, 'back on the list');

  async function run() {
    await waitFor(() => P() && S().dev && S().queue.length === asks.length && $$('.card').length === asks.length, 'list rendered');
    assert($$('.card').length === asks.length, 'list shows every open ask');
    assert(S().dev.pubkey.length > 80 && C.unb64(S().dev.pubkey).length === 65, 'device generated a P-256 key (65-byte SEC1 point)');
    await waitFor(() => h.registers.length >= 1, 'registered with daemon');
    assert(h.registers[0].signer_id === 'phone@iphone' && h.registers[0].pubkey === S().dev.pubkey, 'registration carries the signer id and pubkey');
    await waitFor(() => S().enrolled === true, 'enrolment detected');
    assert($$('.card')[0].getAttribute('data-open') === 'p-appr', 'urgent approval sorts first');
    assert(!window.__xss, 'no markdown XSS on the list');

    // yes/no with markdown detail
    click($('[data-open="p-yesno"]'));
    await waitFor(() => $('#p-q'), 'ask view opened');
    assert(text('#p-q') === 'Ship the release?', 'question shown');
    assert($('#p-detail code') && !window.__xss && !$('#p-detail script'), 'markdown rendered safely');
    assert(button('yes').textContent === 'Ship it' && button('no').textContent === 'Hold', 'custom yes/no labels');
    click(button('note'));
    await waitFor(() => $('#p-note'), 'note field open');
    $('#p-note').value = 'from the phone';
    click(button('yes'));
    await waitFor(() => h.answers.length === 1, 'answer posted');
    assert(h.answers[0].id === 'p-yesno' && h.answers[0].body.answer === 'Ship it' && h.answers[0].body.via === 'phone' && h.answers[0].body.note === 'from the phone', 'yes/no answer with note, via phone');
    await waitFor(() => S().view === 'list', 'back to the list after answering');

    // choices
    click($('[data-open="p-choice"]'));
    await waitFor(() => $('#p-q'), 'choice ask opened');
    assert(button('confirm').disabled, 'confirm disabled until a choice is picked');
    click($$('.opts button')[1]);
    await waitFor(() => !button('confirm').disabled, 'confirm enabled after pick');
    click(button('confirm'));
    await waitFor(() => h.answers.length === 2, 'choice answer posted');
    assert(h.answers[1].body.answer === 'indian', 'choice value posted');
    await listAgain();

    // approval: long question, ttl choice, once, note, renews -> signed envelope
    click($('[data-open="p-appr"]'));
    await waitFor(() => $('#p-q'), 'approval opened');
    assert($('#p-q').classList.contains('xlong'), 'long question uses the compact face');
    assert($('.lock.ok') && /phone@iphone/.test(text('.lock.ok')), 'approval shows the signer');
    const ttlBtns = $$('.ttl button[data-act="ttl"]');
    assert(ttlBtns[0].classList.contains('sel') && /10m/.test(ttlBtns[0].textContent), 'asked TTL preselected');
    click(ttlBtns[2]);
    await sleep(20);
    assert($$('.ttl button[data-act="ttl"]')[2].classList.contains('sel'), 'TTL can be changed');
    click($('input[data-act="once"]'));
    await sleep(20);
    click(button('note'));
    await waitFor(() => $('#p-note'), 'note open');
    $('#p-note').value = 'staging only';
    click(button('yes'));
    await waitFor(() => h.grants.length === 1 && S().grant, 'grant posted and shown');
    const g = h.grants[0];
    assert(g.alg === 'es256' && /^ta-[a-z2-7]{6}$/.test(g.id), 'envelope has alg es256 and a ta- id');
    assert(g.record.question === LONG && g.record.asked_by === 'fixture-bot' && g.record.ask_id === 'p-appr', 'record carries the verbatim question, agent and ask id');
    assert(g.record.decision === 'approve' && g.record.note === 'staging only' && g.record.once === true && g.record.renews === 'ta-old001' && g.record.answer === undefined, 'record decision/note/once/renews, no answer');
    const ttl = (Date.parse(g.record.expires_at) - Date.parse(g.record.issued_at)) / 1000;
    assert(ttl === 14400, 'expires_at = issued_at + chosen TTL (4h)');
    assert(/Z$/.test(g.record.issued_at) && !/\./.test(g.record.issued_at), 'timestamps at second precision');
    const verdict = await C.verifyEnvelope(g, S().dev.pubkey);
    assert(verdict === null, 'envelope verifies with the device public key (hash, id, signature)');
    assert(text('#p-grant .gid') === g.id, 'grant overlay shows the id');
    click(button('grant-ok'));
    await waitFor(() => !S().grant && S().view === 'list', 'grant overlay closed');

    // approval with choices: deny needs no choice; approve needs one; collision retry
    click($('[data-open="p-appr-choice"]'));
    await waitFor(() => $('#p-q'), 'approval with choices opened');
    assert(button('yes').disabled && !button('no').disabled, 'approve disabled without a choice, deny enabled');
    click($$('.opts button')[1]);
    await waitFor(() => !button('yes').disabled, 'approve enabled after pick');
    h.grantFailOnce = true;
    click(button('yes'));
    await waitFor(() => h.grants.length === 3 && S().grant, 'collision retried and grant shown', 8000);
    assert(h.grants[2].record.answer === 'plan B' && h.grants[2].record.once === undefined && h.grants[2].id !== h.grants[1].id || h.grants[2].record.issued_at !== h.grants[1].record.issued_at, 'retry re-signed with the choice; fresh issued_at');
    click(button('grant-ok'));
    await waitFor(() => S().view === 'list', 'list again');

    // numeric
    click($('[data-open="p-num"]'));
    await waitFor(() => $('#p-num'), 'numeric opened');
    assert(text('#p-num').startsWith('4'), 'numeric starts at the default');
    click(button('plus')); click(button('plus'));
    await sleep(20);
    assert(text('#p-num').startsWith('6'), 'plus steps the value');
    click(button('confirm'));
    await waitFor(() => h.answers.length === 3, 'numeric posted');
    assert(h.answers[2].body.answer === '6 asks', 'numeric answer with unit');
    await listAgain();

    // freetext with suggestion
    click($('[data-open="p-text"]'));
    await waitFor(() => $('#p-text'), 'freetext opened');
    click(button('confirm'));
    await sleep(20);
    assert(h.answers.length === 3 && /type an answer/.test(text('#p-msg')), 'empty text refused');
    click($('button[data-act="suggest"]'));
    await sleep(20);
    assert($('#p-text').value === 'kiosk', 'suggestion fills the field');
    click(button('confirm'));
    await waitFor(() => h.answers.length === 4, 'text posted');
    assert(h.answers[3].body.answer === 'kiosk', 'text answer posted');
    await listAgain();

    // form
    click($('[data-open="p-form"]'));
    await waitFor(() => $('.formq'), 'form opened');
    assert(/1 \/ 2/.test(text('.formq .step')), 'form step 1 of 2');
    click(button('yes'));
    await waitFor(() => /2 \/ 2/.test(text('.formq .step')), 'form advanced');
    click(button('back-step'));
    await waitFor(() => /1 \/ 2/.test(text('.formq .step')), 'back a step');
    click(button('no'));
    await waitFor(() => /2 \/ 2/.test(text('.formq .step')), 'form advanced again');
    click(button('confirm'));
    await waitFor(() => h.answers.length === 5, 'form posted');
    assert(h.answers[4].body.answer === JSON.stringify({ ok: 'No', count: '3' }), 'form posted as JSON of sub-answers');
    await listAgain();

    // snooze / dismiss arm
    click($('[data-open="p-ack"]'));
    await waitFor(() => $('#p-q'), 'ack opened');
    click(button('dismiss'));
    await sleep(20);
    assert(h.dismisses.length === 0 && /again/.test(text('#p-msg')) && button('dismiss').classList.contains('arm'), 'first dismiss only arms');
    click(button('snooze'));
    await waitFor(() => h.snoozes.length === 1, 'snoozed');
    assert(h.snoozes[0].id === 'p-ack' && h.snoozes[0].body.minutes === 15 && h.dismisses.length === 0, 'snooze posted, dismiss disarmed');
    await listAgain();

    // live events: an ask answered elsewhere while open
    h.sources[0].emit({ event: 'notification_created', notification: ask('p-live', 8, { question: 'Live one', question_type: 'yesno' }) });
    await waitFor(() => $('[data-open="p-live"]'), 'SSE created ask appears');
    click($('[data-open="p-live"]'));
    await waitFor(() => text('#p-q') === 'Live one', 'live ask opened');
    h.sources[0].emit({ event: 'notification_answered', envelope: { id: 'p-live', answer_label: 'Yes', via: 'console' } });
    await waitFor(() => S().view === 'list' && S().closed, 'closed elsewhere returns to the list with a notice');
    assert(/no longer open/.test(text('.lock.warn')) && /console/.test(text('.lock.warn')), 'notice names who answered');
    click(button('closed-ok'));

    // opening an id that is already answered (push tap on an old banner)
    await P().open('p-gone');
    await waitFor(() => S().closed && S().closed.id === 'p-gone', 'unknown id fetched and reported');
    assert(/answered: Yes/.test(S().closed.what), 'already-answered notice');

    // settings
    click($('[data-nav="settings"]'));
    await waitFor(() => $('#p-pubkey'), 'settings opened');
    assert(text('#p-pubkey') === S().dev.pubkey, 'settings shows the pubkey');
    assert(/ta signers add --id phone@iphone/.test(document.body.textContent), 'settings shows the pin command');
    assert(!window.__xss, 'still no XSS');
    assert(h.uncaught.length === 0, 'no uncaught errors');
  }

  run().then(() => {
    document.getElementById('regression-title').textContent = `Phone regression: passed (${h.passes.length} checks)`;
    document.body.dataset.regression = 'passed';
  }).catch((e) => {
    record(e.message, 'fail');
    document.getElementById('regression-title').textContent = `Phone regression: failed after ${h.passes.length} checks`;
    document.body.dataset.regression = 'failed';
  });
}());
