(function (root, factory) {
  'use strict';

  const api = factory();
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  if (root) root.SjbisAgentVisibility = api;
})(typeof window !== 'undefined' ? window : globalThis, function () {
  'use strict';

  const DAY_MS = 24 * 60 * 60 * 1000;
  const DEFAULT_ACTIVITY_PRESET = '7d';
  const ACTIVITY_WINDOWS_MS = Object.freeze({
    '1d': DAY_MS,
    '7d': 7 * DAY_MS,
    '30d': 30 * DAY_MS,
    all: null,
  });

  function normalizeActivityPreset(value) {
    return Object.prototype.hasOwnProperty.call(ACTIVITY_WINDOWS_MS, value)
      ? value
      : DEFAULT_ACTIVITY_PRESET;
  }

  function timestampMilliseconds(value) {
    if (value instanceof Date) {
      const timestamp = value.getTime();
      return Number.isFinite(timestamp) ? timestamp : null;
    }
    if (typeof value === 'number') return Number.isFinite(value) ? value : null;
    if (typeof value !== 'string' || value.trim() === '') return null;

    const timestamp = Date.parse(value);
    return Number.isFinite(timestamp) ? timestamp : null;
  }

  function sourceKeySet(sourceKeys) {
    if (sourceKeys == null) return new Set();
    if (typeof sourceKeys === 'string') return new Set([sourceKeys]);
    if (Array.isArray(sourceKeys) || typeof sourceKeys[Symbol.iterator] === 'function') {
      return new Set(Array.from(sourceKeys, String));
    }
    if (typeof sourceKeys === 'object') {
      return new Set(Object.keys(sourceKeys).filter((key) => sourceKeys[key]));
    }
    return new Set();
  }

  function partitionSourceKeys(
    sourceSummaries,
    currentSourceKeys,
    selectedFilter,
    activityPreset,
    now
  ) {
    const summaries = sourceSummaries && typeof sourceSummaries === 'object'
      ? sourceSummaries
      : {};
    const preset = normalizeActivityPreset(activityPreset);
    const activityWindowMs = ACTIVITY_WINDOWS_MS[preset];
    const parsedNow = timestampMilliseconds(now);
    const nowMs = parsedNow === null ? Date.now() : parsedNow;
    const cutoffMs = activityWindowMs === null ? null : nowMs - activityWindowMs;
    const protectedSourceKeys = sourceKeySet(currentSourceKeys);
    const selectedSourceKey = selectedFilter == null ? null : String(selectedFilter);
    const visible = [];
    const hidden = [];

    for (const [sourceKey, summary] of Object.entries(summaries)) {
      const currentOrSelected = protectedSourceKeys.has(sourceKey)
        || selectedSourceKey === sourceKey;
      const hasOpenNotification = Boolean(summary && summary.has_open_notification);
      const lastActivityMs = timestampMilliseconds(summary && summary.last_activity_at);
      const isStale = cutoffMs !== null
        && lastActivityMs !== null
        && lastActivityMs < cutoffMs;

      if (currentOrSelected || hasOpenNotification || !isStale) {
        visible.push(sourceKey);
      } else {
        hidden.push(sourceKey);
      }
    }

    return { visible, hidden, hiddenCount: hidden.length };
  }

  return Object.freeze({
    ACTIVITY_WINDOWS_MS,
    DEFAULT_ACTIVITY_PRESET,
    normalizeActivityPreset,
    partitionSourceKeys,
  });
});
