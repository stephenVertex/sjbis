(function () {
  'use strict';

  const SECOND_MS = 1000;
  const MINUTE_MS = 60 * SECOND_MS;
  const HOUR_MS = 60 * MINUTE_MS;
  const DAY_MS = 24 * HOUR_MS;
  const LEGACY_AGE = /^-(\d+):([0-5]\d):([0-5]\d)$/;
  const ISO_TIMESTAMP = /^\d{4}-\d{2}-\d{2}T/;

  const pacificFormatter = new Intl.DateTimeFormat('en-US', {
    timeZone: 'America/Los_Angeles',
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
    timeZoneName: 'short',
  });

  function parseAbsoluteTimestamp(value) {
    if (value instanceof Date) {
      const timestamp = value.getTime();
      return Number.isFinite(timestamp) ? timestamp : null;
    }

    if (typeof value === 'number') {
      return Number.isFinite(value) ? value : null;
    }

    if (typeof value !== 'string') return null;
    const trimmed = value.trim();
    if (!ISO_TIMESTAMP.test(trimmed)) return null;

    const timestamp = Date.parse(trimmed);
    return Number.isFinite(timestamp) ? timestamp : null;
  }

  function ageMilliseconds(value, nowMs) {
    if (typeof value === 'string') {
      const legacy = value.trim().match(LEGACY_AGE);
      if (legacy) {
        return (Number(legacy[1]) * HOUR_MS)
          + (Number(legacy[2]) * MINUTE_MS)
          + (Number(legacy[3]) * SECOND_MS);
      }
    }

    const timestamp = parseAbsoluteTimestamp(value);
    if (timestamp === null) return null;
    return Math.max(0, nowMs - timestamp);
  }

  function formatAge(value, options) {
    const opts = options || {};
    const nowMs = Number.isFinite(opts.nowMs) ? opts.nowMs : Date.now();
    const ageMs = ageMilliseconds(value, nowMs);
    if (ageMs === null || ageMs < SECOND_MS) return 'just now';

    let amount;
    if (ageMs < MINUTE_MS) {
      amount = `${Math.floor(ageMs / SECOND_MS)}s`;
    } else if (ageMs < HOUR_MS) {
      const minutes = Math.floor(ageMs / MINUTE_MS);
      const seconds = Math.floor((ageMs % MINUTE_MS) / SECOND_MS);
      amount = `${minutes}m ${seconds}s`;
    } else if (ageMs < DAY_MS) {
      const hours = Math.floor(ageMs / HOUR_MS);
      const minutes = Math.floor((ageMs % HOUR_MS) / MINUTE_MS);
      amount = `${hours}h ${minutes}m`;
    } else {
      const days = Math.floor(ageMs / DAY_MS);
      const hours = Math.floor((ageMs % DAY_MS) / HOUR_MS);
      amount = `${days}d ${hours}h`;
    }

    const suffix = opts.suffix == null ? 'ago' : String(opts.suffix).trim();
    return suffix ? `${amount} ${suffix}` : amount;
  }

  function formatPacificDateTime(value) {
    const timestamp = parseAbsoluteTimestamp(value);
    return timestamp === null ? '' : pacificFormatter.format(timestamp);
  }

  function toIsoTimestamp(value) {
    const timestamp = parseAbsoluteTimestamp(value);
    return timestamp === null ? '' : new Date(timestamp).toISOString();
  }

  window.SjbisTime = Object.freeze({
    formatAge,
    formatPacificDateTime,
    toIsoTimestamp,
  });
})();
