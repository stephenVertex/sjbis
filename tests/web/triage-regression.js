(function () {
  'use strict';

  window.history.replaceState({}, '', `${window.location.pathname}?view=triage`);

  const now = '2026-09-27T01:00:00Z';
  const hash = (character) => character.repeat(64);
  const freshness = (state, reason, candidatePaths) => ({
    state,
    reason: reason || null,
    candidate_paths: candidatePaths || [],
  });
  const item = (id, state, extra) => ({
    id,
    source_kind: 'path',
    path: `notes/${id}_analysis.md`,
    source: null,
    markdown: `# ${id}\n\nStored **Markdown** for [${id}](note://${id}).`,
    content_sha256: hash(id[0]),
    captured_at: now,
    freshness: freshness(state),
    latest_revision: null,
    ...(extra || {}),
  });

  const queue = (id, name, status, complete, counts) => ({
    id,
    name,
    root: `/srv/triage/${id}`,
    strip_suffix: '_analysis.md',
    source_spec: { kind: 'glob', patterns: ['notes/**/*_analysis.md'] },
    status,
    complete,
    created_at: now,
    updated_at: now,
    closed_at: status === 'closed' ? now : null,
    counts,
  });

  const openCatalog = [
    item('alpha', 'current'),
    item('beta', 'missing', {
      freshness: freshness('missing', 'The captured path no longer exists.'),
    }),
    item('gamma', 'content_changed', {
      freshness: freshness('content_changed', 'The source bytes changed after capture.'),
    }),
    item('delta', 'ambiguous', {
      freshness: freshness('ambiguous', 'Two current files match the captured content.', [
        'notes/delta-renamed-a_analysis.md',
        'notes/delta-renamed-b_analysis.md',
      ]),
    }),
    item('epsilon', 'current'),
    item('zeta', 'current', {
      source_kind: 'inline',
      path: null,
      source: { note_id: 'ys-fixture-zeta', title: 'Inline source fixture' },
    }),
  ];

  const completeRevision = {
    event_id: 1,
    queue_id: 'q-complete',
    item_id: 'complete-item',
    revision: 1,
    verdict: 'schedule',
    target: null,
    content_sha256: hash('c'),
    decided_at: now,
  };
  const queueDetails = {
    'q-open': {
      queue: queue('q-open', 'September note review', 'open', false, {
        total: 6, decided: 0, stale: 3, ambiguous: 1,
      }),
      catalog: openCatalog,
    },
    'q-closed': {
      queue: queue('q-closed', 'Archived review', 'closed', false, {
        total: 1, decided: 0, stale: 0, ambiguous: 0,
      }),
      catalog: [item('archived', 'current')],
    },
    'q-complete': {
      queue: queue('q-complete', 'Finished but open', 'open', true, {
        total: 1, decided: 1, stale: 0, ambiguous: 0,
      }),
      catalog: [item('complete-item', 'current', { latest_revision: completeRevision })],
    },
  };

  let eventId = 10;
  let failNextDecision = false;
  const revisions = {};
  const harness = {
    failures: [],
    passes: [],
    decisionBodies: [],
    requests: [],
  };
  window.__triageRegression = harness;

  function clone(value) {
    return JSON.parse(JSON.stringify(value));
  }

  function queueList() {
    return ['q-open', 'q-closed', 'q-complete'].map((id) => clone(queueDetails[id].queue));
  }

  function recalculate(queueId) {
    const detail = queueDetails[queueId];
    detail.queue.counts.decided = detail.catalog.filter((candidate) => candidate.latest_revision?.verdict).length;
    detail.queue.complete = detail.queue.counts.decided === detail.queue.counts.total;
    detail.queue.updated_at = new Date().toISOString();
  }

  function json(body, status) {
    return new Response(JSON.stringify(body), {
      status: status || 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  window.fetch = async (input, init) => {
    const url = new URL(String(input && input.url ? input.url : input), window.location.href);
    const method = (init && init.method) || 'GET';
    harness.requests.push(`${method} ${url.pathname}`);

    if (url.pathname.endsWith('/state') && method === 'GET') {
      return json({ notifications: [], history: [], rules: [], agents: {}, version: 'triage-regression' });
    }
    if (url.pathname.endsWith('/triage/queues') && method === 'GET') {
      return json(queueList());
    }

    const decisionMatch = url.pathname.match(/\/triage\/queues\/([^/]+)\/items\/([^/]+)\/decision$/);
    if (decisionMatch && method === 'PATCH') {
      const queueId = decodeURIComponent(decisionMatch[1]);
      const itemId = decodeURIComponent(decisionMatch[2]);
      const body = JSON.parse(init.body);
      harness.decisionBodies.push({ queueId, itemId, body });
      if (failNextDecision) {
        failNextDecision = false;
        return json({ error: 'fixture rejected this decision' }, 409);
      }
      const catalogItem = queueDetails[queueId].catalog.find((candidate) => candidate.id === itemId);
      const revisionNumber = (revisions[`${queueId}/${itemId}`] || 0) + 1;
      revisions[`${queueId}/${itemId}`] = revisionNumber;
      const revision = {
        event_id: eventId++,
        queue_id: queueId,
        item_id: itemId,
        revision: revisionNumber,
        verdict: body.verdict,
        target: body.target,
        content_sha256: catalogItem.content_sha256,
        decided_at: new Date().toISOString(),
      };
      catalogItem.latest_revision = revision;
      recalculate(queueId);
      return json(clone(revision));
    }

    const statusMatch = url.pathname.match(/\/triage\/queues\/([^/]+)\/(close|reopen)$/);
    if (statusMatch && method === 'POST') {
      const queueId = decodeURIComponent(statusMatch[1]);
      const status = statusMatch[2] === 'close' ? 'closed' : 'open';
      queueDetails[queueId].queue.status = status;
      queueDetails[queueId].queue.closed_at = status === 'closed' ? new Date().toISOString() : null;
      return json(clone(queueDetails[queueId].queue));
    }

    const detailMatch = url.pathname.match(/\/triage\/queues\/([^/]+)$/);
    if (detailMatch && method === 'GET') {
      const queueId = decodeURIComponent(detailMatch[1]);
      return queueDetails[queueId]
        ? json(clone(queueDetails[queueId]))
        : json({ error: `unknown fixture queue: ${queueId}` }, 404);
    }

    if (/\/(dismiss|snooze)\//.test(url.pathname) || url.pathname.endsWith('/rules')) {
      return json({ ok: true });
    }
    return json({ error: `Unexpected fixture request: ${method} ${url.pathname}` }, 500);
  };

  window.EventSource = class FixtureEventSource {
    constructor() {
      setTimeout(() => this.onopen?.(new Event('open')), 0);
    }
    close() {}
  };

  function record(message, className) {
    const entry = document.createElement('li');
    entry.className = className;
    entry.textContent = message;
    document.getElementById('regression-results').appendChild(entry);
  }

  function assert(condition, message) {
    if (!condition) throw new Error(message);
    harness.passes.push(message);
    record(message, 'pass');
  }

  function waitFor(getValue, message, timeoutMs) {
    const deadline = Date.now() + (timeoutMs || 10000);
    return new Promise((resolve, reject) => {
      function poll() {
        let value;
        try {
          value = getValue();
        } catch (error) {
          reject(error);
          return;
        }
        if (value) resolve(value);
        else if (Date.now() >= deadline) reject(new Error(`Timed out: ${message}`));
        else setTimeout(poll, 25);
      }
      poll();
    });
  }

  function currentItemId() {
    return document.querySelector('.triage-review')?.dataset.itemId;
  }

  function clickVerdict(verdict) {
    document.querySelector(`[data-verdict="${verdict}"]`).click();
  }

  function press(key, target) {
    (target || window).dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
  }

  async function waitForItem(itemId) {
    return waitFor(() => currentItemId() === itemId && document.querySelector('.triage-markdown'), `item ${itemId} should open`);
  }

  async function run() {
    await waitForItem('alpha');
    assert(document.querySelectorAll('.triage-queue-card').length === 3, 'multiple queue summaries render');
    assert(document.querySelector('.triage-markdown strong').textContent === 'Markdown', 'stored Markdown renders as JSX');
    assert(document.querySelector('.triage-evidence').textContent.includes(hash('a')), 'full snapshot hash renders');
    assert(document.querySelector('.triage-evidence').textContent.includes('Captured path'), 'source and capture evidence render');
    assert(document.querySelectorAll('.triage-sequence-item.triage-freshness-current').length > 0, 'current items have a distinct freshness marker');
    assert(document.querySelectorAll('.triage-sequence-item.triage-freshness-missing').length === 1, 'missing items have a distinct freshness marker');
    assert(document.querySelectorAll('.triage-sequence-item.triage-freshness-content_changed').length === 1, 'changed items have a distinct freshness marker');
    assert(document.querySelectorAll('.triage-sequence-item.triage-freshness-ambiguous').length === 1, 'ambiguous items have a distinct freshness marker');

    clickVerdict('schedule');
    await waitForItem('beta');
    assert(document.querySelector('[data-testid="triage-progress"]').textContent.includes('1 of 6'), 'progress refreshes from queue detail after a verdict');
    assert(document.querySelector('.triage-freshness-missing'), 'missing evidence remains visible with the stored snapshot');

    clickVerdict('delete');
    await waitForItem('gamma');
    assert(document.querySelector('.triage-freshness-content_changed'), 'changed evidence remains visible with the stored snapshot');

    clickVerdict('needs_replan');
    await waitForItem('delta');
    assert(document.querySelectorAll('.triage-candidates li').length === 2, 'ambiguity candidate paths render');

    press('4');
    const target = await waitFor(() => (
      document.activeElement?.id === 'triage-merge-target' ? document.activeElement : null
    ), 'merge target picker should receive focus');
    press('ArrowDown', target);
    await waitFor(() => target.value === 'beta', 'keyboard should change the merge target');
    press('Enter', target);
    await waitForItem('epsilon');

    clickVerdict('leave_captured');
    await waitForItem('zeta');
    assert(
      JSON.stringify(harness.decisionBodies.slice(0, 5).map((entry) => entry.body.verdict)) ===
        JSON.stringify(['schedule', 'delete', 'needs_replan', 'merge_into', 'leave_captured']),
      'all five verdicts post their stable wire values'
    );
    assert(harness.decisionBodies[3].body.target === 'beta', 'merge_into posts a catalog-backed target');

    press('[');
    await waitForItem('epsilon');
    document.querySelector('.triage-record-actions button').click();
    await waitFor(() => document.querySelector('.triage-verdict-panel'), 'revise action should restore verdict controls');
    clickVerdict('schedule');
    await waitForItem('zeta');
    press('[');
    await waitForItem('epsilon');
    assert(document.querySelector('.triage-decision-record').textContent.includes('revision 2'), 'revision history advances when a verdict is replaced');
    document.querySelector('.triage-record-actions .is-danger').click();
    await waitFor(() => document.querySelector('[data-testid="triage-progress"]').textContent.includes('4 of 6'), 'clear should reduce returned progress');
    assert(currentItemId() === 'epsilon', 'clear keeps the current item selected');
    assert(document.querySelector('.triage-verdict-panel'), 'a cleared decision is immediately editable');

    press(']');
    await waitForItem('zeta');
    failNextDecision = true;
    clickVerdict('delete');
    await waitFor(() => document.querySelector('.triage-error')?.textContent.includes('fixture rejected this decision'), 'server diagnostics should render');
    assert(currentItemId() === 'zeta', 'a failed mutation does not advance the queue');

    document.querySelector('.triage-queue-actions .is-close').click();
    await waitFor(() => document.querySelector('[data-testid="queue-status"]')?.textContent.trim() === 'Closed', 'close should refresh queue detail');
    assert(document.querySelector('.triage-closed-notice'), 'closed queues explain that review remains available');
    assert(Array.from(document.querySelectorAll('.triage-verdict button')).every((button) => button.disabled), 'closed queues disable decision mutation controls');
    assert(document.querySelector('.triage-export'), 'closed queues keep export available');
    document.querySelector('.triage-queue-actions .is-reopen').click();
    await waitFor(() => document.querySelector('[data-testid="queue-status"]')?.textContent.trim() !== 'Closed', 'reopen should restore the open state');
    assert(document.querySelector('.triage-verdict-panel'), 'reopening restores mutation controls');

    document.querySelector('[data-queue-id="q-complete"]').click();
    await waitForItem('complete-item');
    assert(document.querySelector('[data-testid="queue-status"]').textContent.trim() === 'Review complete', 'complete open queues are not labeled closed');
    assert(document.querySelector('.triage-record-actions button:not(:disabled)'), 'complete open queues still allow revision');

    const mutations = harness.requests.filter((request) => /^(PATCH|POST)/.test(request));
    assert(mutations.every((request) => request.includes('/triage/')), 'triage mutations stay on isolated triage routes');
    document.body.dataset.regression = 'passed';
    document.getElementById('regression-title').textContent = `Triage regression: passed (${harness.passes.length})`;
    harness.done = true;
  }

  harness.promise = new Promise((resolve) => {
    window.addEventListener('load', () => {
      run().then(resolve).catch((error) => {
        harness.failures.push(error);
        harness.done = true;
        document.body.dataset.regression = 'failed';
        document.getElementById('regression-title').textContent = 'Triage regression: failed';
        record(error.stack || error.message || String(error), 'fail');
        resolve();
      });
    });
  });
}());
