(function () {
  'use strict';

  const ORIGINAL_EXCEPTION = "TypeError: Cannot read properties of undefined (reading 'length')";
  const FORCED_EXCEPTION = 'Forced Focus regression failure';
  const PLAIN_ID = 'sjbis-focus-plain';
  const CANONICAL_ID = 'sjbis-focus-canonical';
  const CRASH_ID = 'sjbis-focus-crash';
  const PLAIN_CHOICES = [
    'Accept suggestion',
    'Keep → plan it',
    'Wontfix / obsolete',
    'Duplicate',
    'Needs discussion',
  ];
  const CANONICALIZED_PLAIN_CHOICES = PLAIN_CHOICES.map((choice) => ({
    value: choice,
    label: choice,
  }));
  const MARKDOWN = [
    '# Triage review: yesod-triage',
    '',
    '> This card needs a disposition decision. Please review the linked evidence and pick one of the five choices.',
    '',
    '## Evidence',
    '',
    '- Link to related note: [ys-yes-24ho](note://ys-yes-24ho)',
    '- Link to another related note: [ys-ays-51x9](note://ys-ays-51x9)',
    '- Context from the mayor\'s run:',
    '  - The card was posted with plain-string choices.',
    '  - The dashboard blanked when the card was clicked.',
  ].join('\n');

  const notifications = [
    {
      id: PLAIN_ID,
      agent_name: 'yesod-triage',
      instance: 'Mayor triage regression',
      sender: 'yesod-triage',
      src: 'yesod-triage · Mayor triage regression',
      question: 'How should ys-yes-24ho be dispositioned?',
      detail_markdown: MARKDOWN,
      question_type: 'multichoice',
      choices: CANONICALIZED_PLAIN_CHOICES,
      urgency: 3,
      blocking: false,
      created_at: '2026-09-22T18:32:20Z',
    },
    {
      id: CANONICAL_ID,
      agent_name: 'yesod-triage',
      sender: 'yesod-triage',
      question: 'Which canonical action should be submitted?',
      detail: 'The visible label intentionally differs from the wire value.',
      question_type: 'multichoice',
      choices: [
        { value: 'triage-accept', label: 'Accept suggestion' },
        { value: 'triage-plan', label: 'Keep and plan it', hint: 'Submit the stable id' },
      ],
      urgency: 2,
      blocking: false,
      created_at: '2026-09-22T18:33:20Z',
    },
    {
      id: CRASH_ID,
      agent_name: 'yesod-triage',
      sender: 'yesod-triage',
      question: 'Force a descendant render exception',
      detail: 'The fixture replaces Focus for this card only.',
      question_type: 'ack',
      urgency: 1,
      blocking: false,
      created_at: '2026-09-22T18:34:20Z',
    },
  ];

  const harness = {
    answers: [],
    expectedErrors: [],
    failures: [],
    passes: [],
    uncaught: [],
  };
  window.__focusRegression = harness;
  localStorage.removeItem('sjbis.historyHidden');

  function captureBrowserError(error) {
    const message = String(error);
    if (message.includes(FORCED_EXCEPTION)) harness.expectedErrors.push(message);
    else harness.uncaught.push(error);
  }

  window.addEventListener('error', (event) => captureBrowserError(event.error || event.message));
  window.addEventListener('unhandledrejection', (event) => captureBrowserError(event.reason));

  const response = (body, status) => new Response(JSON.stringify(body), {
    status: status || 200,
    headers: { 'Content-Type': 'application/json' },
  });

  window.fetch = async (input, init) => {
    const url = String(input && input.url ? input.url : input);
    const method = (init && init.method) || 'GET';
    if (url.endsWith('/state') && method === 'GET') {
      return response({
        notifications,
        history: [],
        rules: [],
        agents: {
          'yesod-triage': { name: 'yesod-triage', glyph: '◈' },
        },
        version: 'focus-regression',
      });
    }
    const answerMatch = url.match(/\/answer\/([^/?]+)$/);
    if (answerMatch && method === 'POST') {
      harness.answers.push({
        id: decodeURIComponent(answerMatch[1]),
        body: JSON.parse(init.body),
      });
      return response({ ok: true });
    }
    if (/\/(dismiss|snooze)\//.test(url) || url.endsWith('/rules')) {
      return response({ ok: true });
    }
    return response({ error: `Unexpected fixture request: ${method} ${url}` }, 500);
  };

  window.EventSource = class FixtureEventSource {
    constructor() {
      setTimeout(() => {
        if (this.onopen) this.onopen(new Event('open'));
      }, 0);
    }

    close() {}
  };

  function installFocusCrash() {
    const focusImplementation = window.Focus;
    window.Focus = function FocusRegressionProxy(props) {
      if (props.n.id === CRASH_ID) throw new Error(FORCED_EXCEPTION);
      return React.createElement(focusImplementation, props);
    };
  }

  function record(message, className) {
    const item = document.createElement('li');
    item.className = className;
    item.textContent = message;
    document.getElementById('regression-results').appendChild(item);
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
        if (value) {
          resolve(value);
        } else if (Date.now() >= deadline) {
          reject(new Error(`Timed out: ${message}`));
        } else {
          setTimeout(poll, 25);
        }
      }
      poll();
    });
  }

  function cardForQuestion(question) {
    return Array.from(document.querySelectorAll('.card')).find((card) => (
      card.querySelector('.q') && card.querySelector('.q').textContent === question
    ));
  }

  function choiceLabels() {
    return Array.from(document.querySelectorAll('.focus .choice .lbl')).map((node) => node.textContent);
  }

  async function closeFocus() {
    document.querySelector('.focus .close').click();
    await waitFor(() => !document.querySelector('.focus'), 'Focus should close');
  }

  async function run() {
    document.getElementById('original-exception').textContent = ORIGINAL_EXCEPTION;
    await waitFor(() => document.querySelectorAll('.card').length === 3, 'controlled cards should render', 15000);
    installFocusCrash();
    assert(document.querySelector('#root .app'), 'dashboard root remains mounted');

    const plainNotification = notifications.find((notification) => notification.id === PLAIN_ID);
    // The live CLI path returns canonical objects; swap in the legacy wire form
    // immediately before Focus opens to exercise its defensive string decoder.
    plainNotification.choices = PLAIN_CHOICES;
    cardForQuestion('How should ys-yes-24ho be dispositioned?').click();
    await waitFor(() => document.querySelector('.focus'), 'plain-string Focus should open');
    assert(JSON.stringify(choiceLabels()) === JSON.stringify(PLAIN_CHOICES), 'plain-string payload renders all five labels in order');
    assert(document.querySelectorAll('.focus .md-detail a').length === 2, 'long markdown renders both note-id links');
    assert(harness.uncaught.length === 0, 'plain-string Focus produces no uncaught browser error');
    plainNotification.choices = CANONICALIZED_PLAIN_CHOICES;
    await closeFocus();

    cardForQuestion('Which canonical action should be submitted?').click();
    await waitFor(() => choiceLabels().includes('Keep and plan it'), 'canonical Focus should open');
    assert(choiceLabels().includes('Accept suggestion'), 'canonical choice labels render');
    Array.from(document.querySelectorAll('.focus .choice')).find((button) => (
      button.querySelector('.lbl').textContent === 'Keep and plan it'
    )).click();
    await waitFor(() => harness.answers.length === 1, 'canonical answer request should be captured');
    assert(harness.answers[0].id === CANONICAL_ID, 'answer targets the focused card id');
    assert(harness.answers[0].body.answer === 'triage-plan', 'answer posts choice.value rather than the visible label');
    assert(harness.uncaught.length === 0, 'canonical Focus produces no uncaught browser error');
    await waitFor(() => !document.querySelector('.focus'), 'answered Focus should close');

    cardForQuestion('Force a descendant render exception').click();
    const fallback = await waitFor(() => document.querySelector('.focus-error[role="alert"]'), 'error fallback should render');
    assert(fallback.textContent.includes(CRASH_ID), 'error fallback includes the card id');
    assert(fallback.textContent.includes(FORCED_EXCEPTION), 'error fallback includes the render exception');
    assert(harness.expectedErrors.length > 0, 'browser reports only the deliberately forced render exception');
    assert(harness.uncaught.length === 0, 'forced render containment produces no unrelated uncaught error');
    assert(document.querySelector('#root .app'), 'forced Focus error does not unmount the dashboard');
    fallback.querySelector('.focus-error-copy button').click();
    await waitFor(() => !document.querySelector('.focus-error'), 'fallback Close should return to the list');
    assert(cardForQuestion('Force a descendant render exception'), 'card list remains usable after closing the fallback');

    cardForQuestion('How should ys-yes-24ho be dispositioned?').click();
    await waitFor(() => choiceLabels().length === 5, 'a normal card should open after the forced failure');
    assert(!document.querySelector('.focus-error'), 'the boundary resets for the next card');
    await closeFocus();

    document.body.dataset.regression = 'passed';
    document.getElementById('regression-title').textContent = `Focus regression: passed (${harness.passes.length})`;
    harness.done = true;
  }

  harness.promise = new Promise((resolve) => {
    window.addEventListener('load', () => {
      run().then(resolve).catch((error) => {
        harness.failures.push(error);
        harness.done = true;
        document.body.dataset.regression = 'failed';
        document.getElementById('regression-title').textContent = 'Focus regression: failed';
        record(error.stack || error.message || String(error), 'fail');
        resolve();
      });
    });
  });
}());
