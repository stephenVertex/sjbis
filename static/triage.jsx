// Deliberate batch-triage review surface. This module only talks to /triage/*
// routes; notification state and SSE remain isolated in app.jsx.
(function () {
  'use strict';

  const VERDICTS = [
    { value: 'schedule', label: 'Schedule', description: 'Keep it and put it on the plan.' },
    { value: 'delete', label: 'Delete', description: 'Remove it as obsolete or unwanted.' },
    { value: 'needs_replan', label: 'Needs replan', description: 'The item is valid, but its plan is not.' },
    { value: 'merge_into', label: 'Merge into', description: 'Fold this item into another catalog item.' },
    { value: 'leave_captured', label: 'Leave captured', description: 'Keep the snapshot without scheduling work.' },
  ];

  const VERDICT_LABELS = Object.fromEntries(VERDICTS.map((verdict) => [verdict.value, verdict.label]));
  const FRESHNESS_LABELS = {
    current: 'Current',
    missing: 'Missing at source',
    content_changed: 'Source changed',
    ambiguous: 'Ambiguous move',
  };

  async function triageRequest(path, options) {
    const response = await fetch(`${API_BASE}${path}`, options);
    const contentType = response.headers.get('content-type') || '';
    const body = contentType.includes('application/json')
      ? await response.json().catch(() => ({}))
      : await response.text().catch(() => '');
    if (!response.ok) {
      const message = body && typeof body === 'object' ? body.error : body;
      throw new Error(message || `Triage request failed (${response.status})`);
    }
    return body;
  }

  function apiTriageQueues() {
    return triageRequest('/triage/queues');
  }

  function apiTriageQueue(queueId) {
    return triageRequest(`/triage/queues/${encodeURIComponent(queueId)}`);
  }

  function apiTriageDecision(queueId, itemId, patch) {
    return triageRequest(
      `/triage/queues/${encodeURIComponent(queueId)}/items/${encodeURIComponent(itemId)}/decision`,
      {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(patch),
      }
    );
  }

  function apiTriageQueueStatus(queueId, action) {
    return triageRequest(`/triage/queues/${encodeURIComponent(queueId)}/${action}`, {
      method: 'POST',
    });
  }

  function formatDate(value) {
    if (!value) return 'unknown';
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(date);
  }

  function queueStatusLabel(queue) {
    if (queue.status === 'closed') return 'Closed';
    if (queue.complete) return 'Review complete';
    return 'Open';
  }

  function queueProgress(queue) {
    if (!queue.counts.total) return 0;
    return Math.round((queue.counts.decided / queue.counts.total) * 100);
  }

  function sourceSpecLabel(sourceSpec) {
    if (!sourceSpec) return 'Unknown source';
    if (sourceSpec.kind === 'glob') return sourceSpec.patterns.join(', ');
    if (sourceSpec.kind === 'json_list') return sourceSpec.path;
    return sourceSpec.kind || 'Unknown source';
  }

  function nextPendingIndex(catalog, currentIndex) {
    for (let offset = 1; offset <= catalog.length; offset += 1) {
      const index = (currentIndex + offset) % catalog.length;
      if (!catalog[index].latest_revision?.verdict) return index;
    }
    return Math.min(currentIndex + 1, catalog.length - 1);
  }

  function QueueCard({ queue, selected, onSelect }) {
    const progress = queueProgress(queue);
    const pending = Math.max(0, queue.counts.total - queue.counts.decided);
    return (
      <button
        type="button"
        className={`triage-queue-card${selected ? ' is-selected' : ''}`}
        onClick={onSelect}
        aria-pressed={selected}
        data-queue-id={queue.id}
      >
        <span className="triage-queue-card-top">
          <strong>{queue.name}</strong>
          <span className={`triage-status status-${queue.status}${queue.complete ? ' is-complete' : ''}`}>
            {queueStatusLabel(queue)}
          </span>
        </span>
        <span className="triage-queue-progress" aria-hidden="true">
          <span style={{ width: `${progress}%` }} />
        </span>
        <span className="triage-queue-card-meta">
          {queue.counts.decided}/{queue.counts.total} reviewed
          <span>{pending} pending</span>
        </span>
        {(queue.counts.stale > 0 || queue.counts.ambiguous > 0) && (
          <span className="triage-queue-card-flags">
            {queue.counts.stale > 0 && <span>{queue.counts.stale} stale</span>}
            {queue.counts.ambiguous > 0 && <span>{queue.counts.ambiguous} ambiguous</span>}
          </span>
        )}
      </button>
    );
  }

  function QueueSummary({ queue }) {
    const pending = Math.max(0, queue.counts.total - queue.counts.decided);
    const progress = queueProgress(queue);
    return (
      <div className="triage-summary" aria-label="Queue progress">
        <div className="triage-summary-copy">
          <span className="triage-eyebrow">Queue progress</span>
          <strong data-testid="triage-progress">
            {queue.counts.decided} of {queue.counts.total} reviewed
          </strong>
        </div>
        <div className="triage-summary-meter" aria-label={`${progress}% complete`}>
          <span style={{ width: `${progress}%` }} />
        </div>
        <div className="triage-summary-stats">
          <span><b>{pending}</b> pending</span>
          <span><b>{queue.counts.stale}</b> stale</span>
          <span><b>{queue.counts.ambiguous}</b> ambiguous</span>
        </div>
      </div>
    );
  }

  function ItemSequence({ catalog, currentIndex, onSelect }) {
    return (
      <div className="triage-sequence" aria-label="Queue item order">
        {catalog.map((item, index) => (
          <button
            type="button"
            key={item.id}
            className={[
              'triage-sequence-item',
              `triage-freshness-${item.freshness.state}`,
              index === currentIndex ? 'is-current' : '',
              item.latest_revision?.verdict ? 'is-decided' : '',
            ].filter(Boolean).join(' ')}
            onClick={() => onSelect(index)}
            aria-label={`${index + 1}. ${item.id}; ${FRESHNESS_LABELS[item.freshness.state] || item.freshness.state}; ${item.latest_revision?.verdict ? 'reviewed' : 'pending'}`}
            aria-current={index === currentIndex ? 'step' : undefined}
            title={item.path || item.id}
          >
            <span>{index + 1}</span>
          </button>
        ))}
      </div>
    );
  }

  function SnapshotEvidence({ item }) {
    const freshness = item.freshness || { state: 'current', candidate_paths: [] };
    const sourceValue = item.source && JSON.stringify(item.source, null, 2);
    return (
      <aside className="triage-evidence" aria-label="Snapshot evidence">
        <div className="triage-evidence-heading">
          <span className="triage-eyebrow">Captured evidence</span>
          <span className={`triage-freshness triage-freshness-${freshness.state}`}>
            {FRESHNESS_LABELS[freshness.state] || freshness.state}
          </span>
        </div>
        {freshness.reason && <p className="triage-freshness-reason">{freshness.reason}</p>}
        <dl>
          <div>
            <dt>Source kind</dt>
            <dd>{item.source_kind}</dd>
          </div>
          <div>
            <dt>{item.path ? 'Captured path' : 'Inline source'}</dt>
            <dd>{item.path || sourceValue || 'No provenance supplied'}</dd>
          </div>
          <div>
            <dt>Captured</dt>
            <dd>{formatDate(item.captured_at)}</dd>
          </div>
          <div>
            <dt>SHA-256</dt>
            <dd><code>{item.content_sha256}</code></dd>
          </div>
        </dl>
        {freshness.candidate_paths && freshness.candidate_paths.length > 0 && (
          <div className="triage-candidates">
            <span className="triage-eyebrow">Possible current paths</span>
            <ul>
              {freshness.candidate_paths.map((path) => <li key={path}><code>{path}</code></li>)}
            </ul>
          </div>
        )}
      </aside>
    );
  }

  function DecisionRecord({ revision, disabled, onRevise, onClear, busy }) {
    if (!revision) return null;
    return (
      <div className="triage-decision-record">
        <div>
          <span className="triage-eyebrow">Latest decision - revision {revision.revision}</span>
          <strong>
            {VERDICT_LABELS[revision.verdict] || 'Cleared'}
            {revision.target && <span> into {revision.target}</span>}
          </strong>
          <span className="triage-decision-time">Saved {formatDate(revision.decided_at)}</span>
        </div>
        {revision.verdict && (
          <div className="triage-record-actions">
            <button type="button" onClick={onRevise} disabled={disabled || busy}>Revise decision</button>
            <button type="button" className="is-danger" onClick={onClear} disabled={disabled || busy}>
              Clear <kbd>X</kbd>
            </button>
          </div>
        )}
      </div>
    );
  }

  function VerdictControls({
    item,
    catalog,
    mergeMode,
    mergeTarget,
    onMergeMode,
    onMergeTarget,
    onVerdict,
    disabled,
    busy,
  }) {
    const targets = catalog.filter((candidate) => candidate.id !== item.id);
    const selectRef = React.useRef(null);

    React.useEffect(() => {
      if (mergeMode) selectRef.current?.focus();
    }, [mergeMode, item.id]);

    const moveTarget = (offset) => {
      if (!targets.length) return;
      const currentIndex = Math.max(0, targets.findIndex((candidate) => candidate.id === mergeTarget));
      const nextIndex = Math.max(0, Math.min(targets.length - 1, currentIndex + offset));
      onMergeTarget(targets[nextIndex].id);
    };

    return (
      <div className="triage-verdict-panel" aria-label="Record a verdict">
        <div className="triage-verdict-heading">
          <div>
            <span className="triage-eyebrow">Your verdict</span>
            <strong>{item.latest_revision?.verdict ? 'Choose a replacement decision' : 'Choose one outcome'}</strong>
          </div>
          <span className="triage-key-hint">Keys 1-5</span>
        </div>
        <div className="triage-verdicts">
          {VERDICTS.map((verdict, index) => (
            <button
              type="button"
              key={verdict.value}
              className={`triage-verdict verdict-${verdict.value}${mergeMode && verdict.value === 'merge_into' ? ' is-active' : ''}`}
              data-verdict={verdict.value}
              disabled={disabled || busy}
              onClick={() => verdict.value === 'merge_into' ? onMergeMode() : onVerdict(verdict.value)}
            >
              <kbd>{index + 1}</kbd>
              <span>
                <strong>{verdict.label}</strong>
                <small>{verdict.description}</small>
              </span>
            </button>
          ))}
        </div>
        {mergeMode && (
          <div className="triage-target-picker">
            <label htmlFor="triage-merge-target">Merge destination</label>
            <select
              ref={selectRef}
              id="triage-merge-target"
              value={mergeTarget}
              disabled={disabled || busy || targets.length === 0}
              onChange={(event) => onMergeTarget(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'ArrowDown') {
                  event.preventDefault();
                  moveTarget(1);
                } else if (event.key === 'ArrowUp') {
                  event.preventDefault();
                  moveTarget(-1);
                } else if (event.key === 'Enter' && mergeTarget) {
                  event.preventDefault();
                  onVerdict('merge_into', mergeTarget);
                }
              }}
            >
              {targets.map((target) => (
                <option key={target.id} value={target.id}>
                  {target.id}{target.path ? ` - ${target.path}` : ''}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="triage-merge-submit"
              disabled={disabled || busy || !mergeTarget}
              onClick={() => onVerdict('merge_into', mergeTarget)}
            >
              Merge into selected <kbd>Enter</kbd>
            </button>
          </div>
        )}
      </div>
    );
  }

  function TriageDashboard({ onExit }) {
    const [queues, setQueues] = React.useState([]);
    const [selectedQueueId, setSelectedQueueId] = React.useState(null);
    const [detail, setDetail] = React.useState(null);
    const [itemIndex, setItemIndex] = React.useState(0);
    const [editing, setEditing] = React.useState(false);
    const [mergeMode, setMergeMode] = React.useState(false);
    const [mergeTarget, setMergeTarget] = React.useState('');
    const [loading, setLoading] = React.useState(true);
    const [busy, setBusy] = React.useState(false);
    const [error, setError] = React.useState('');
    const reviewHeadingRef = React.useRef(null);

    const queue = detail?.queue;
    const catalog = detail?.catalog || [];
    const item = catalog[itemIndex] || null;
    const closed = queue?.status === 'closed';
    const showVerdictControls = item && (!item.latest_revision?.verdict || editing);
    const decisionEditable = showVerdictControls && !closed;

    const loadQueues = React.useCallback(async () => {
      const nextQueues = await apiTriageQueues();
      setQueues(nextQueues);
      setSelectedQueueId((current) => (
        current && nextQueues.some((candidate) => candidate.id === current)
          ? current
          : nextQueues[0]?.id || null
      ));
      return nextQueues;
    }, []);

    React.useEffect(() => {
      setLoading(true);
      loadQueues()
        .catch((loadError) => setError(loadError.message))
        .finally(() => setLoading(false));
    }, [loadQueues]);

    React.useEffect(() => {
      if (!selectedQueueId) {
        setDetail(null);
        return;
      }
      let cancelled = false;
      setLoading(true);
      setError('');
      apiTriageQueue(selectedQueueId)
        .then((nextDetail) => {
          if (cancelled) return;
          setDetail(nextDetail);
          setItemIndex(0);
        })
        .catch((loadError) => !cancelled && setError(loadError.message))
        .finally(() => !cancelled && setLoading(false));
      return () => { cancelled = true; };
    }, [selectedQueueId]);

    React.useEffect(() => {
      if (!item) return;
      setEditing(false);
      setMergeMode(false);
      const targets = catalog.filter((candidate) => candidate.id !== item.id);
      const savedTarget = item.latest_revision?.target;
      setMergeTarget(
        savedTarget && targets.some((candidate) => candidate.id === savedTarget)
          ? savedTarget
          : targets[0]?.id || ''
      );
      setError('');
      reviewHeadingRef.current?.focus();
    }, [item?.id, selectedQueueId]);

    const refreshAfterMutation = async (currentItemId, advance) => {
      const [nextDetail, nextQueues] = await Promise.all([
        apiTriageQueue(selectedQueueId),
        apiTriageQueues(),
      ]);
      const currentIndex = Math.max(0, nextDetail.catalog.findIndex((candidate) => candidate.id === currentItemId));
      const nextIndex = advance ? nextPendingIndex(nextDetail.catalog, currentIndex) : currentIndex;
      setDetail(nextDetail);
      setQueues(nextQueues);
      setItemIndex(nextIndex);
      setEditing(false);
      setMergeMode(false);
    };

    const recordVerdict = async (verdict, target) => {
      if (!item || closed || busy) return;
      const currentItemId = item.id;
      setBusy(true);
      setError('');
      try {
        await apiTriageDecision(selectedQueueId, currentItemId, {
          verdict,
          target: verdict === 'merge_into' ? target : null,
        });
        await refreshAfterMutation(currentItemId, true);
      } catch (mutationError) {
        setError(mutationError.message);
      } finally {
        setBusy(false);
      }
    };

    const clearDecision = async () => {
      if (!item || closed || busy) return;
      const currentItemId = item.id;
      setBusy(true);
      setError('');
      try {
        await apiTriageDecision(selectedQueueId, currentItemId, { verdict: null, target: null });
        await refreshAfterMutation(currentItemId, false);
      } catch (mutationError) {
        setError(mutationError.message);
      } finally {
        setBusy(false);
      }
    };

    const changeQueueStatus = async () => {
      if (!queue || busy) return;
      const action = queue.status === 'closed' ? 'reopen' : 'close';
      setBusy(true);
      setError('');
      try {
        await apiTriageQueueStatus(queue.id, action);
        await refreshAfterMutation(item?.id || '', false);
      } catch (mutationError) {
        setError(mutationError.message);
      } finally {
        setBusy(false);
      }
    };

    const moveItem = React.useCallback((offset) => {
      if (!catalog.length) return;
      setItemIndex((current) => Math.max(0, Math.min(catalog.length - 1, current + offset)));
    }, [catalog.length]);

    React.useEffect(() => {
      const isTyping = (element) => element?.matches?.('input, textarea, select, [contenteditable="true"]');
      const onKeyDown = (event) => {
        if (!event.key || isTyping(event.target) || busy) return;
        if (event.key === 'Escape') {
          event.preventDefault();
          onExit?.();
          return;
        }
        if (event.key === '[' || event.key === 'ArrowLeft' || event.key.toLowerCase() === 'k') {
          event.preventDefault();
          moveItem(-1);
          return;
        }
        if (event.key === ']' || event.key === 'ArrowRight' || event.key.toLowerCase() === 'j') {
          event.preventDefault();
          moveItem(1);
          return;
        }
        if (!item || closed) return;
        if (event.key.toLowerCase() === 'r' && item.latest_revision?.verdict) {
          event.preventDefault();
          setEditing(true);
          return;
        }
        if (event.key.toLowerCase() === 'x' && item.latest_revision?.verdict) {
          event.preventDefault();
          clearDecision();
          return;
        }
        if (!decisionEditable || !/^[1-5]$/.test(event.key)) return;
        event.preventDefault();
        const verdict = VERDICTS[Number(event.key) - 1].value;
        if (verdict === 'merge_into') setMergeMode(true);
        else recordVerdict(verdict);
      };
      window.addEventListener('keydown', onKeyDown);
      return () => window.removeEventListener('keydown', onKeyDown);
    }, [busy, closed, decisionEditable, item, moveItem, onExit, clearDecision, recordVerdict]);

    if (loading && !detail) {
      return <main className="triage-shell triage-loading">Loading triage queues...</main>;
    }

    if (!queues.length) {
      return (
        <main className="triage-shell triage-empty">
          <span className="triage-eyebrow">Batch review</span>
          <h1>No triage queues yet</h1>
          <p>Create one with the CLI, then return here to review its captured snapshots.</p>
          {error && <div className="triage-error" role="alert">{error}</div>}
        </main>
      );
    }

    return (
      <main className="triage-shell">
        <aside className="triage-queue-rail" aria-label="Triage queues">
          <div className="triage-rail-heading">
            <div>
              <span className="triage-eyebrow">Batch review</span>
              <h1>Triage queues</h1>
            </div>
            <select
              className="triage-mobile-select"
              value={selectedQueueId || ''}
              onChange={(event) => setSelectedQueueId(event.target.value)}
              aria-label="Select triage queue"
            >
              {queues.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.name}</option>)}
            </select>
          </div>
          <div className="triage-queue-list">
            {queues.map((candidate) => (
              <QueueCard
                key={candidate.id}
                queue={candidate}
                selected={candidate.id === selectedQueueId}
                onSelect={() => setSelectedQueueId(candidate.id)}
              />
            ))}
          </div>
        </aside>

        {queue && (
          <section className="triage-workspace">
            <header className="triage-workspace-header">
              <div className="triage-queue-title">
                <span className={`triage-status status-${queue.status}${queue.complete ? ' is-complete' : ''}`} data-testid="queue-status">
                  {queueStatusLabel(queue)}
                </span>
                <div>
                  <h2>{queue.name}</h2>
                  <p><code>{queue.root}</code> / {sourceSpecLabel(queue.source_spec)}</p>
                </div>
              </div>
              <div className="triage-queue-actions">
                <a
                  className="triage-export"
                  href={`${API_BASE}/triage/queues/${encodeURIComponent(queue.id)}/export`}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Export JSON
                </a>
                <button
                  type="button"
                  className={queue.status === 'closed' ? 'is-reopen' : 'is-close'}
                  onClick={changeQueueStatus}
                  disabled={busy}
                >
                  {queue.status === 'closed' ? 'Reopen queue' : 'Close queue'}
                </button>
              </div>
            </header>

            <QueueSummary queue={queue} />
            <ItemSequence catalog={catalog} currentIndex={itemIndex} onSelect={setItemIndex} />

            {error && <div className="triage-error" role="alert">{error}</div>}
            {closed && (
              <div className="triage-closed-notice" role="status">
                This queue is closed. Review and export remain available; reopen it to change decisions.
              </div>
            )}

            {item ? (
              <article className="triage-review" data-item-id={item.id}>
                <div className="triage-review-nav">
                  <button type="button" onClick={() => moveItem(-1)} disabled={itemIndex === 0} aria-label="Previous item">
                    <kbd>[</kbd> Previous
                  </button>
                  <span>{itemIndex + 1} / {catalog.length}</span>
                  <button type="button" onClick={() => moveItem(1)} disabled={itemIndex === catalog.length - 1} aria-label="Next item">
                    Next <kbd>]</kbd>
                  </button>
                </div>

                <div className="triage-review-grid">
                  <div className="triage-snapshot">
                    <div className="triage-item-heading">
                      <span className="triage-eyebrow">Stored Markdown snapshot</span>
                      <h2 ref={reviewHeadingRef} tabIndex="-1">{item.id}</h2>
                      {item.path && <p>{item.path}</p>}
                    </div>
                    <div className="triage-markdown focus-detail-markdown">
                      {window.SjbisMarkdown.render(item.markdown)}
                    </div>
                  </div>
                  <SnapshotEvidence item={item} />
                </div>

                <DecisionRecord
                  revision={item.latest_revision}
                  disabled={closed}
                  busy={busy}
                  onRevise={() => setEditing(true)}
                  onClear={clearDecision}
                />

                {showVerdictControls && (
                  <VerdictControls
                    item={item}
                    catalog={catalog}
                    mergeMode={mergeMode}
                    mergeTarget={mergeTarget}
                    onMergeMode={() => setMergeMode(true)}
                    onMergeTarget={setMergeTarget}
                    onVerdict={recordVerdict}
                    disabled={closed}
                    busy={busy}
                  />
                )}
              </article>
            ) : (
              <div className="triage-empty-catalog">This queue has no captured items.</div>
            )}
          </section>
        )}
      </main>
    );
  }

  Object.assign(window, {
    TriageDashboard,
    SjbisTriageApi: {
      listQueues: apiTriageQueues,
      getQueue: apiTriageQueue,
      recordDecision: apiTriageDecision,
      setQueueStatus: apiTriageQueueStatus,
    },
  });
}());
