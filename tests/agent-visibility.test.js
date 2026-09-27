'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const {
  DEFAULT_ACTIVITY_PRESET,
  partitionSourceKeys,
} = require('../static/agent-visibility.js');

const NOW_MS = Date.parse('2026-09-27T12:00:00Z');
const DAY_MS = 24 * 60 * 60 * 1000;
const daysAgo = (days) => new Date(NOW_MS - (days * DAY_MS)).toISOString();

test('finite windows hide only valid activity strictly older than the cutoff', () => {
  const summaries = {
    recent: { last_activity_at: daysAgo(2) },
    'at-cutoff': { last_activity_at: daysAgo(7) },
    stale: { last_activity_at: daysAgo(8) },
    null: { last_activity_at: null },
    absent: {},
    malformed: { last_activity_at: 'not-a-timestamp' },
    future: { last_activity_at: new Date(NOW_MS + DAY_MS).toISOString() },
  };
  const original = structuredClone(summaries);

  const result = partitionSourceKeys(summaries, [], null, '7d', NOW_MS);

  assert.deepEqual(result, {
    visible: ['recent', 'at-cutoff', 'null', 'absent', 'malformed', 'future'],
    hidden: ['stale'],
    hiddenCount: 1,
  });
  assert.deepEqual(summaries, original);
});

test('open, currently displayed, and selected stale sources stay visible', () => {
  const summaries = {
    'server-open': { last_activity_at: daysAgo(90), has_open_notification: true },
    displayed: { last_activity_at: daysAgo(90) },
    selected: { last_activity_at: daysAgo(90) },
    hidden: { last_activity_at: daysAgo(90) },
  };

  const result = partitionSourceKeys(
    summaries,
    new Set(['displayed']),
    'selected',
    '1d',
    NOW_MS
  );

  assert.deepEqual(result, {
    visible: ['server-open', 'displayed', 'selected'],
    hidden: ['hidden'],
    hiddenCount: 1,
  });
});

test('presets produce deterministic partitions and show-all hides nothing', () => {
  const summaries = {
    recent: { last_activity_at: daysAgo(0.5) },
    week: { last_activity_at: daysAgo(5) },
    month: { last_activity_at: daysAgo(20) },
    old: { last_activity_at: daysAgo(31) },
  };

  assert.deepEqual(
    partitionSourceKeys(summaries, null, null, '1d', NOW_MS).hidden,
    ['week', 'month', 'old']
  );
  assert.deepEqual(
    partitionSourceKeys(summaries, null, null, '7d', NOW_MS).hidden,
    ['month', 'old']
  );
  assert.deepEqual(
    partitionSourceKeys(summaries, null, null, '30d', NOW_MS).hidden,
    ['old']
  );
  assert.deepEqual(
    partitionSourceKeys(summaries, null, null, 'all', NOW_MS),
    {
      visible: ['recent', 'week', 'month', 'old'],
      hidden: [],
      hiddenCount: 0,
    }
  );
  assert.deepEqual(
    partitionSourceKeys(summaries, null, null, '7d', NOW_MS),
    partitionSourceKeys(summaries, null, null, '7d', NOW_MS)
  );
  assert.equal(DEFAULT_ACTIVITY_PRESET, '7d');
});

test('the same policy is exposed as a browser global', () => {
  const source = fs.readFileSync(
    path.join(__dirname, '..', 'static', 'agent-visibility.js'),
    'utf8'
  );
  const browser = { window: {} };

  vm.runInNewContext(source, browser);

  assert.equal(typeof browser.window.SjbisAgentVisibility.partitionSourceKeys, 'function');
});
