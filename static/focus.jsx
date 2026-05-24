// Focus mode renderers — one per question type.
// Each renderer returns the interactive surface inside the focus card.
// All call onAnswer(value) to dismiss and burst.

const TYPE_LABEL = {
  yesno: 'Yes / No',
  multichoice: 'Multi-choice',
  freetext: 'Free text reply',
  numeric: 'Numeric',
  file: 'File upload',
  diff: 'Approve / reject',
  ack: 'Acknowledge',
  picklist: 'Pick from list',
  schedule: 'Schedule',
};

// Convert "-00:01:42" to "1m 42s ago"
function fmtSentAt(s) {
  if (!s) return '';
  if (s.includes('T')) {
    // ISO date string from API
    const d = new Date(s);
    const now = new Date();
    const diff = Math.floor((now - d) / 1000);
    if (diff < 60) return `${diff}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    return `${Math.floor(diff / 3600)}h ${Math.floor((diff % 3600) / 60)}m ago`;
  }
  const [h, m, sec] = s.replace('-', '').split(':').map(Number);
  if (h) return `${h}h ${m}m ago`;
  if (m) return `${m}m ${sec}s ago`;
  return `${sec}s ago`;
}

// Normalize API snake_case fields to camelCase expected by renderers
function normalizeNotif(n) {
  return {
    ...n,
    type: n.question_type || n.type || 'ack',
    agent: n.agent_name || n.agent,
    yesLabel: n.yesLabel || n.yes_label,
    noLabel: n.noLabel || n.no_label,
    ackLabel: n.ackLabel || n.ack_label,
    defaultValue: n.defaultValue !== undefined ? n.defaultValue : n.default_value,
    deadlineMs: n.deadlineMs || (n.deadline ? Math.max(0, new Date(n.deadline) - Date.now()) : undefined),
    sentAt: n.sentAt || n.created_at || '',
  };
}

// ── Countdown widget ────────────────────────────────────────────────────
function Countdown({ ms, urgent }) {
  const [remain, setRemain] = React.useState(ms);
  React.useEffect(() => {
    const t = setInterval(() => setRemain((r) => Math.max(0, r - 1000)), 1000);
    return () => clearInterval(t);
  }, []);
  const m = Math.floor(remain / 60000);
  const s = Math.floor((remain % 60000) / 1000);
  return (
    <div className={'countdown' + (urgent || remain < 60000 ? ' urgent' : '')}>
      <span className="num">{String(m).padStart(2, '0')}:{String(s).padStart(2, '0')}</span>
      <span style={{ opacity: 0.6 }}>left</span>
    </div>
  );
}

// ── Renderers ───────────────────────────────────────────────────────────

// Map keys → handler. Skips when typing in a textarea/input.
function useFocusKeys(handlers, deps) {
  React.useEffect(() => {
    const isTyping = (el) => {
      if (!el) return false;
      const tag = el.tagName;
      return tag === 'INPUT' || tag === 'TEXTAREA' || el.isContentEditable;
    };
    const onKey = (e) => {
      if (isTyping(e.target) && !e.metaKey && !e.ctrlKey) return;
      if (!e.key) return; // Guard against undefined key (composite/dead keys)
      for (const spec of handlers) {
        if (spec.match(e)) {
          e.preventDefault();
          spec.fn(e);
          return;
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}

function YesNoRenderer({ n, onAnswer }) {
  // iMessage-sourced notifications get an "edit before send" affordance —
  // the literal answer text gets sent back over the wire, so let the user
  // customize ("Yes — be home by 9 ok?") instead of forcing the canned label.
  const isChat = (n.agent_name || n.agent) === 'fam';
  const [editing, setEditing] = React.useState(null);
  // {label, text, k} or null. k disambiguates Yes vs No when both share a key.
  const startEdit = (label) => setEditing({ label, text: label, k: label });
  useFocusKeys([
    { match: (e) => !editing && (e.key === 'y' && !e.shiftKey || e.key === 'Enter'),
      fn: () => onAnswer(n.yesLabel || 'Yes') },
    { match: (e) => !editing && e.key === 'n' && !e.shiftKey,
      fn: () => onAnswer(n.noLabel || 'No') },
    { match: (e) => !!editing && e.key === 'Escape',
      fn: () => setEditing(null) },
  ], [n, onAnswer, editing]);
  if (editing) {
    return (
      <EditBox
        title={`Reply to ${n.sender} via iMessage`}
        text={editing.text}
        onChange={(t) => setEditing({ ...editing, text: t })}
        onCancel={() => setEditing(null)}
        onSend={() => onAnswer(editing.text)}
        sendLabel="Send via iMessage"
      />
    );
  }
  return (
    <div className="yesno">
      <button className="bigbtn yes" onClick={() => onAnswer(n.yesLabel || 'Yes')}>
        {isChat && <EditPen onClick={(e) => { e.stopPropagation(); startEdit(n.yesLabel || 'Yes'); }} dark />}
        <span className="k">Y · ⏎</span>
        {n.yesLabel || 'Yes'}
      </button>
      <button className="bigbtn no" onClick={() => onAnswer(n.noLabel || 'No')}>
        {isChat && <EditPen onClick={(e) => { e.stopPropagation(); startEdit(n.noLabel || 'No'); }} />}
        <span className="k">N</span>
        {n.noLabel || 'No'}
      </button>
    </div>
  );
}

// ── Edit-before-send inline editor (iMessage flow) ──────────────────────
function EditBox({ title, text, onChange, onCancel, onSend, sendLabel }) {
  const ref = React.useRef(null);
  React.useEffect(() => {
    ref.current?.focus();
    ref.current?.setSelectionRange(text.length, text.length);
  }, []);
  return (
    <div className="editbox">
      <div className="hd">
        <span className="bub">✎</span>
        <span>{title}</span>
      </div>
      <textarea
        ref={ref}
        value={text}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); onSend(); }
          else if (e.key === 'Escape') { e.preventDefault(); onCancel(); }
        }}
      />
      <div className="row">
        <span className="meta"><strong>{text.length}</strong> chars · sent verbatim over iMessage</span>
        <div className="actions">
          <button type="button" className="btn" onClick={onCancel}>
            Cancel <span className="k">esc</span>
          </button>
          <button type="button" className="btn primary" onClick={onSend} disabled={!text.trim()}>
            {sendLabel || 'Send'} <span className="k">⌘⏎</span>
          </button>
        </div>
      </div>
    </div>
  );
}

// Pencil icon used to open EditBox from a choice/yesno tile
function EditPen({ onClick, dark }) {
  return (
    <span
      className="edit"
      role="button"
      title="Edit before sending"
      onClick={onClick}
      style={dark ? { background: 'rgba(7,8,12,0.18)', color: 'rgba(7,8,12,0.6)',
                      borderColor: 'rgba(7,8,12,0.25)' } : undefined}
    >✎</span>
  );
}

function MultiChoiceRenderer({ n, onAnswer }) {
  const isChat = (n.agent_name || n.agent) === 'fam';
  const [sel, setSel] = React.useState(null);
  const [editing, setEditing] = React.useState(null);
  const cols = n.choices.length === 3 ? 'three' : n.choices.length === 4 ? 'four' : '';
  const pick = (c) => { setSel(c.value); setTimeout(() => onAnswer(c.label), 220); };
  const startEdit = (label) => setEditing({ text: label });
  const startCustom = () => setEditing({ text: '' });
  useFocusKeys(
    [
      ...n.choices.map((c, i) => ({
        match: (e) => !editing && e.key === String(i + 1),
        fn: () => pick(c),
      })),
      { match: (e) => !!editing && e.key === 'Escape',
        fn: () => setEditing(null) },
    ],
    [n, onAnswer, editing]
  );
  if (editing) {
    return (
      <EditBox
        title={`Reply to ${n.sender} via iMessage`}
        text={editing.text}
        onChange={(t) => setEditing({ text: t })}
        onCancel={() => setEditing(null)}
        onSend={() => onAnswer(editing.text)}
        sendLabel="Send via iMessage"
      />
    );
  }
  return (
    <>
      <div className={`choices ${cols}`}>
        {n.choices.map((c, i) => (
          <button
            key={c.value}
            className={`choice ${sel === c.value ? 'selected' : ''}`}
            onClick={() => pick(c)}
          >
            <span className="k">{i + 1}</span>
            {isChat && (
              <EditPen onClick={(e) => { e.stopPropagation(); startEdit(c.label); }} />
            )}
            <span className="lbl">{c.label}</span>
            {c.hint && <span className="hint">{c.hint}</span>}
          </button>
        ))}
      </div>
      {isChat && (
        <button type="button" className="custom-reply" onClick={startCustom}>
          <span className="pen">✎</span>
          Write a custom reply…
        </button>
      )}
    </>
  );
}

function FreeTextRenderer({ n, onAnswer }) {
  const [text, setText] = React.useState('');
  const ref = React.useRef(null);
  React.useEffect(() => { ref.current?.focus(); }, []);
  useFocusKeys([
    { match: (e) => e.key === 'Enter' && (e.metaKey || e.ctrlKey),
      fn: () => text.trim() && onAnswer(text.trim()) },
    // 1-N selects suggestion (only when not typing — modifier required if focused on textarea)
    ...(n.suggestions || []).map((s, i) => ({
      match: (e) => e.key === String(i + 1) && e.altKey,
      fn: () => setText(s),
    })),
  ], [n, text, onAnswer]);
  return (
    <>
      <div className="composer">
        <textarea
          ref={ref}
          value={text}
          placeholder={n.placeholder}
          onChange={(e) => setText(e.target.value)}
        />
        {n.suggestions && (
          <div className="suggest">
            {n.suggestions.map((s, i) => (
              <div key={i} className="sg" onClick={() => setText(s)}>
                <span style={{
                  fontFamily: 'var(--mono)', fontSize: 10, opacity: 0.5, marginRight: 6,
                }}>⌥{i + 1}</span>
                {s}
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="action-row">
        <button className="btn-action ghost" onClick={() => onAnswer('(skipped)')}>
          Skip
        </button>
        <button
          className="btn-action primary"
          disabled={!text.trim()}
          onClick={() => onAnswer(text.trim() || '(empty)')}
        >
          Send reply <span className="k">⌘ ⏎</span>
        </button>
      </div>
    </>
  );
}

function NumericRenderer({ n, onAnswer }) {
  const [v, setV] = React.useState(n.defaultValue ?? n.min ?? 0);
  const trackRef = React.useRef(null);
  const dragging = React.useRef(false);
  useFocusKeys([
    { match: (e) => e.key === 'ArrowRight' || e.key === '+' || e.key === '=' || e.key === 'l' && !e.shiftKey || e.key === 'j' && !e.shiftKey,
      fn: () => setV((x) => Math.min(n.max, x + n.step)) },
    { match: (e) => e.key === 'ArrowLeft' || e.key === '-' || e.key === '_' || e.key === 'h' && !e.shiftKey || e.key === 'k' && !e.shiftKey,
      fn: () => setV((x) => Math.max(n.min, x - n.step)) },
    { match: (e) => e.key === 'Enter',
      fn: () => onAnswer(`${v} ${n.unit}`) },
  ], [n, v, onAnswer]);

  const pct = ((v - n.min) / (n.max - n.min)) * 100;
  const setFromX = (x) => {
    const r = trackRef.current.getBoundingClientRect();
    const p = Math.max(0, Math.min(1, (x - r.left) / r.width));
    const raw = n.min + p * (n.max - n.min);
    const stepped = Math.round(raw / n.step) * n.step;
    setV(Math.max(n.min, Math.min(n.max, stepped)));
  };
  const onDown = (e) => {
    dragging.current = true;
    setFromX(e.clientX);
    const move = (ev) => dragging.current && setFromX(ev.clientX);
    const up = () => {
      dragging.current = false;
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return (
    <>
      <div className="dial">
        <div className="value">{v}</div>
        <div className="unit">{n.unit}</div>
        <div className="track" ref={trackRef} onPointerDown={onDown}>
          <div className="fill" style={{ width: `calc(${pct}% - 4px + 4px)` }} />
          <div className="thumb" style={{ left: `${pct}%` }}>{v}</div>
        </div>
        <div className="ticks">
          <span>{n.min}</span>
          <span>{Math.round((n.min + n.max) / 2)}</span>
          <span>{n.max}</span>
        </div>
      </div>
      <div className="action-row">
        <button className="btn-action ghost" onClick={() => onAnswer('(skipped)')}>Skip</button>
        <button className="btn-action primary" onClick={() => onAnswer(`${v} ${n.unit}`)}>
          Send {v} {n.unit} <span className="k">⏎</span>
        </button>
      </div>
    </>
  );
}

function FileRenderer({ n, onAnswer }) {
  const [over, setOver] = React.useState(false);
  const [file, setFile] = React.useState(null);
  const inputRef = React.useRef(null);
  const handle = (f) => {
    setFile(f);
    setTimeout(() => onAnswer(`Uploaded: ${f.name}`), 600);
  };
  return (
    <>
      <div
        className={'drop' + (over ? ' over' : '')}
        onClick={() => inputRef.current?.click()}
        onDragOver={(e) => { e.preventDefault(); setOver(true); }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          if (e.dataTransfer.files[0]) handle(e.dataTransfer.files[0]);
        }}
      >
        <div className="ic">{file ? '✓' : '↓'}</div>
        <div className="big">{file ? file.name : 'Drop the file here'}</div>
        <div className="small">{file ? 'Uploading…' : `or click to browse · ${n.accept}`}</div>
        <input
          ref={inputRef}
          type="file" accept={n.accept}
          style={{ display: 'none' }}
          onChange={(e) => e.target.files[0] && handle(e.target.files[0])}
        />
      </div>
      <div className="action-row">
        <button className="btn-action ghost" onClick={() => onAnswer('(skipped)')}>
          Skip for now
        </button>
      </div>
    </>
  );
}

function DiffRenderer({ n, onAnswer }) {
  useFocusKeys([
    { match: (e) => (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) || e.key === 'a' && !e.shiftKey,
      fn: () => onAnswer('Approved') },
    { match: (e) => e.key === 'r' && !e.shiftKey,
      fn: () => onAnswer('Rejected') },
  ], [n, onAnswer]);
  const lines = n.diff || [];
  return (
    <>
      <div className="diff">
        {lines.length > 0 ? lines.map((l, i) => (
          <div key={i} className={`line ${l.kind}`}>{l.text}</div>
        )) : (
          <div className="line ctx" style={{ opacity: 0.7, fontStyle: 'italic' }}>
            No diff preview provided — see full details above.
          </div>
        )}
      </div>
      <div className="action-row">
        <button className="btn-action danger" onClick={() => onAnswer('Rejected')}>
          Reject <span className="k">R</span>
        </button>
        <button className="btn-action primary" onClick={() => onAnswer('Approved')}>
          Approve & merge <span className="k">A · ⌘⏎</span>
        </button>
      </div>
    </>
  );
}

function AckRenderer({ n, onAnswer }) {
  useFocusKeys([
    { match: (e) => e.key === 'Enter' || e.key === ' ',
      fn: () => onAnswer(n.ackLabel || 'Acknowledged') },
  ], [n, onAnswer]);
  return (
    <div className="ack-area">
      <button className="ack-btn" onClick={() => onAnswer(n.ackLabel || 'Acknowledged')}>
        {n.ackLabel || 'Got it'}
      </button>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-3)',
                    letterSpacing: '0.08em', textTransform: 'uppercase' }}>
        Press ⏎ or space
      </div>
    </div>
  );
}

function PicklistRenderer({ n, onAnswer }) {
  const [q, setQ] = React.useState('');
  const [cursor, setCursor] = React.useState(0);
  const items = n.items.filter((i) =>
    !q || (i.title + ' ' + i.meta).toLowerCase().includes(q.toLowerCase()));
  React.useEffect(() => { if (cursor >= items.length) setCursor(Math.max(0, items.length - 1)); }, [items.length, cursor]);
  useFocusKeys([
    { match: (e) => e.key === 'j' && !e.shiftKey && !document.activeElement?.matches('input,textarea'),
      fn: () => setCursor((c) => Math.min(items.length - 1, c + 1)) },
    { match: (e) => e.key === 'k' && !e.shiftKey && !document.activeElement?.matches('input,textarea'),
      fn: () => setCursor((c) => Math.max(0, c - 1)) },
    { match: (e) => e.key === 'ArrowDown', fn: () => setCursor((c) => Math.min(items.length - 1, c + 1)) },
    { match: (e) => e.key === 'ArrowUp',   fn: () => setCursor((c) => Math.max(0, c - 1)) },
    { match: (e) => e.key === 'Enter',
      fn: () => items[cursor] && onAnswer(items[cursor].title) },
  ], [items, cursor, onAnswer]);
  return (
    <>
      <div className="picklist">
        <input
          className="search"
          autoFocus
          value={q}
          placeholder="Search…"
          onChange={(e) => setQ(e.target.value)}
        />
        <div className="items">
          {items.map((it, i) => (
            <div
              key={it.id}
              className={`pl-item ${i === cursor ? 'selected' : ''}`}
              onClick={() => { setCursor(i); }}
              onDoubleClick={() => onAnswer(it.title)}
            >
              <span className="t">{it.title}</span>
              <span className="m">{it.meta}</span>
            </div>
          ))}
        </div>
      </div>
      <div className="action-row">
        <button className="btn-action ghost" onClick={() => onAnswer('(skipped)')}>Skip</button>
        <button
          className="btn-action primary"
          disabled={!items[cursor]}
          onClick={() => items[cursor] && onAnswer(items[cursor].title)}
        >
          Select <span className="k">⏎</span>
        </button>
      </div>
    </>
  );
}

function ScheduleRenderer({ n, onAnswer }) {
  const enabledIdx = n.slots.map((s, i) => s.disabled ? -1 : i).filter((i) => i >= 0);
  const [sel, setSel] = React.useState(enabledIdx[0] ?? null);
  useFocusKeys([
    ...n.slots.map((s, i) => ({
      match: (e) => e.key === String(i + 1) && !s.disabled,
      fn: () => setSel(i),
    })),
    { match: (e) => e.key === 'Enter' && sel !== null,
      fn: () => onAnswer(`${n.slots[sel].day} · ${n.slots[sel].time}`) },
  ], [n, sel, onAnswer]);
  return (
    <>
      <div className="slots">
        {n.slots.map((s, i) => (
          <div
            key={i}
            className={'slot' + (s.disabled ? ' disabled' : '') + (sel === i ? ' selected' : '')}
            onClick={() => !s.disabled && setSel(i)}
          >
            {!s.disabled && <span className="k">{i + 1}</span>}
            <span className="day">{s.day}</span>
            <span className="time">{s.time}</span>
            {s.reason && <span className="reason">{s.reason}</span>}
          </div>
        ))}
      </div>
      <div className="action-row">
        <button className="btn-action ghost" onClick={() => onAnswer('(skipped)')}>Skip</button>
        <button
          className="btn-action primary"
          disabled={sel === null}
          onClick={() => onAnswer(`${n.slots[sel].day} · ${n.slots[sel].time}`)}
        >
          Confirm <span className="k">⏎</span>
        </button>
      </div>
    </>
  );
}

const RENDERERS = {
  yesno: YesNoRenderer,
  multichoice: MultiChoiceRenderer,
  freetext: FreeTextRenderer,
  numeric: NumericRenderer,
  file: FileRenderer,
  diff: DiffRenderer,
  ack: AckRenderer,
  picklist: PicklistRenderer,
  schedule: ScheduleRenderer,
};

// ── Snooze picker ─────────────────────────────────────────────────────────

function SnoozePicker({ n, onSnooze, onClose }) {
  const [custom, setCustom] = React.useState('');
  const [error, setError] = React.useState(null);
  const ref = React.useRef(null);

  const deadline = n.deadline ? new Date(n.deadline).getTime() : null;
  const now = Date.now();
  const maxMinutes = deadline ? Math.floor((deadline - now) / 60000) : null;

  const presets = [
    { label: '5m', minutes: 5, key: '1' },
    { label: '15m', minutes: 15, key: '2' },
    { label: '30m', minutes: 30, key: '3' },
    { label: '1h', minutes: 60, key: '4' },
    { label: '4h', minutes: 240, key: '5' },
  ];

  const isDisabled = (minutes) => maxMinutes !== null && minutes > maxMinutes;

  const doSnooze = (minutes) => {
    if (isDisabled(minutes)) {
      setError(`Cannot snooze past auto-approve deadline. Max: ${maxMinutes}m`);
      return;
    }
    onSnooze(minutes);
  };

  React.useEffect(() => {
    ref.current?.focus();
    const onKey = (e) => {
      if (e.key === 'Escape') { e.preventDefault(); onClose(); }
      const preset = presets.find((p) => p.key === e.key);
      if (preset && !isDisabled(preset.minutes)) {
        e.preventDefault();
        doSnooze(preset.minutes);
      }
      if (e.key === 'Enter' && custom) {
        e.preventDefault();
        const val = parseInt(custom, 10);
        if (val && val > 0) doSnooze(val);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [custom, onClose, onSnooze]);

  return (
    <>
      <div className="focus-backdrop" onClick={onClose} />
      <div className="focus snooze-overlay" style={{ '--agent': window.agentColor ? window.agentColor(n.agent_name || n.agent) : '#C7F33D' }}>
        <div className="focus-hd">
          <div className="meta">
            <div className="label">Snooze — <strong>{n.question}</strong></div>
            {maxMinutes !== null && (
              <div className="sender">Auto-approves in {maxMinutes}m — snooze cannot exceed this</div>
            )}
          </div>
          <button className="close" onClick={onClose}>✕</button>
        </div>
        <div className="focus-body">
          <div className="snooze-presets">
            {presets.map((p) => (
              <button
                key={p.key}
                className={`snooze-pill ${isDisabled(p.minutes) ? 'disabled' : ''}`}
                disabled={isDisabled(p.minutes)}
                onClick={() => doSnooze(p.minutes)}
              >
                <span className="k">{p.key}</span>
                {p.label}
              </button>
            ))}
          </div>
          <div className="snooze-custom">
            <input
              ref={ref}
              type="number"
              min="1"
              max={maxMinutes ?? undefined}
              value={custom}
              placeholder={maxMinutes ? `Custom (max ${maxMinutes}m)` : 'Custom minutes'}
              onChange={(e) => { setCustom(e.target.value); setError(null); }}
              onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); const val = parseInt(custom, 10); if (val && val > 0) doSnooze(val); } }}
            />
            <button className="btn-action primary" disabled={!custom || parseInt(custom, 10) <= 0 || (maxMinutes !== null && parseInt(custom, 10) > maxMinutes)} onClick={() => doSnooze(parseInt(custom, 10))}>
              Snooze <span className="k">⏎</span>
            </button>
          </div>
          {error && <div className="snooze-error">{error}</div>}
          <div style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-3)', marginTop: 12 }}>
            Press <strong>1–5</strong> for preset, type + <strong>Enter</strong> for custom, <strong>Esc</strong> to cancel
          </div>
        </div>
      </div>
    </>
  );
}

// ── Focus shell ─────────────────────────────────────────────────────────

function Focus({ n, onClose, onAnswer, onSnooze }) {
  const nn = normalizeNotif(n);
  const agent = window.AGENTS ? (window.AGENTS[nn.agent] || { glyph: '◐', name: nn.agent }) : { glyph: '◐', name: nn.agent };
  const Renderer = RENDERERS[nn.type] || AckRenderer;
  const color = window.agentColor ? window.agentColor(nn.agent) : '#C7F33D';
  const [snoozing, setSnoozing] = React.useState(false);
  const [note, setNote] = React.useState('');
  const [showNote, setShowNote] = React.useState(false);
  const noteRef = React.useRef(null);

  const handleAnswer = (val) => onAnswer(val, note.trim() || null);

  React.useEffect(() => {
    const onKey = (e) => {
      if (e.key === 'Escape') { if (snoozing) { setSnoozing(false); } else { onClose(); } }
      if (!snoozing && e.key === 's' && !e.shiftKey) {
        // Don't trigger snooze if typing in an input/textarea
        const tag = e.target?.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || e.target?.isContentEditable) return;
        e.preventDefault();
        setSnoozing(true);
      }
      if (e.key === 'N' && !e.target?.matches('input,textarea')) {
        e.preventDefault();
        setShowNote((v) => !v);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, snoozing]);

  React.useEffect(() => {
    if (showNote) noteRef.current?.focus();
  }, [showNote]);

  if (snoozing) {
    return (
      <SnoozePicker
        n={n}
        onSnooze={(minutes) => { setSnoozing(false); onSnooze(minutes); }}
        onClose={() => setSnoozing(false)}
      />
    );
  }

  return (
    <>
      <div className="focus-backdrop" onClick={onClose} />
      <div className="focus" style={{ '--agent': color }}>
        <div className="focus-hd">
          <div className="glyph">{agent.glyph}</div>
          <div className="meta">
            <div className="label">
              <strong>{nn.sender}</strong> via {agent.name} · {fmtSentAt(nn.sentAt)} · {TYPE_LABEL[nn.type] || nn.type}
            </div>
            <div className="sender" style={{ marginTop: 2 }}>
              Urgency {nn.urgency}/5 · {nn.blocking ? 'Blocking' : 'Non-blocking'}
            </div>
          </div>
          {nn.deadlineMs > 0 && <Countdown ms={nn.deadlineMs} urgent={nn.urgency >= 4} />}
          <button className="close" onClick={onClose}>✕</button>
        </div>
        <div className="focus-body">
          <h2 className="focus-q">{nn.question}</h2>
          {nn.detail && <p className="focus-detail">{nn.detail}</p>}
          <Renderer n={nn} onAnswer={handleAnswer} />
          <div className="note-composer">
            {showNote ? (
              <>
                <textarea
                  ref={noteRef}
                  className="note-area"
                  placeholder="Optional note for the agent…"
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Escape') { e.preventDefault(); setShowNote(false); }
                  }}
                />
                <div className="note-meta">
                  <span>{note.length} chars</span>
                  <button className="note-close" onClick={() => setShowNote(false)}>Hide note <span className="k">⇧N</span></button>
                </div>
              </>
            ) : (
              <button className="note-toggle" onClick={() => setShowNote(true)}>
                <span>✎</span> Add note <span className="k">⇧N</span>
              </button>
            )}
          </div>
        </div>
      </div>
    </>
  );
}

// Burst overlay (celebration on answer)
function Burst({ text, color, onDone }) {
  React.useEffect(() => {
    const t = setTimeout(onDone, 1100);
    return () => clearTimeout(t);
  }, [onDone]);
  const confetti = React.useMemo(() => {
    const colors = ['#C7F33D', '#FF3D7F', '#5BD4FF', '#FFB341', '#B89DFF'];
    return Array.from({ length: 28 }).map((_, i) => ({
      dx: (Math.random() - 0.5) * 800,
      dy: (Math.random() - 0.3) * 600,
      r: (Math.random() - 0.5) * 720,
      c: colors[i % colors.length],
      delay: Math.random() * 0.1,
      x: 50 + (Math.random() - 0.5) * 20,
      y: 50 + (Math.random() - 0.5) * 10,
    }));
  }, []);
  return (
    <div className="burst">
      {confetti.map((c, i) => (
        <div
          key={i}
          className="confetti"
          style={{
            '--dx': c.dx + 'px',
            '--dy': c.dy + 'px',
            '--r': c.r + 'deg',
            '--c': c.c,
            left: c.x + '%',
            top: c.y + '%',
            animationDelay: c.delay + 's',
          }}
        />
      ))}
      <div className="msg" style={{ background: color || 'var(--lime)' }}>{text}</div>
    </div>
  );
}

Object.assign(window, { Focus, Burst, TYPE_LABEL, fmtSentAt });
