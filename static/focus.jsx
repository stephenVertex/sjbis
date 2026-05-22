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
  const [h, m, sec] = s.replace('-', '').split(':').map(Number);
  if (h) return `${h}h ${m}m ago`;
  if (m) return `${m}m ${sec}s ago`;
  return `${sec}s ago`;
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
  const isChat = n.agent === 'fam';
  const [editing, setEditing] = React.useState(null);
  // {label, text, k} or null. k disambiguates Yes vs No when both share a key.
  const startEdit = (label) => setEditing({ label, text: label, k: label });
  useFocusKeys([
    { match: (e) => !editing && (e.key.toLowerCase() === 'y' || e.key === 'Enter'),
      fn: () => onAnswer(n.yesLabel || 'Yes') },
    { match: (e) => !editing && e.key.toLowerCase() === 'n',
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
  const isChat = n.agent === 'fam';
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
    { match: (e) => e.key === 'ArrowRight' || e.key === '+' || e.key === '=' || e.key.toLowerCase() === 'l' || e.key.toLowerCase() === 'j',
      fn: () => setV((x) => Math.min(n.max, x + n.step)) },
    { match: (e) => e.key === 'ArrowLeft' || e.key === '-' || e.key === '_' || e.key.toLowerCase() === 'h' || e.key.toLowerCase() === 'k',
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
        <button className="btn-action ghost" onClick={() => onAnswer('(skipped)')}>Skip for now</button>
      </div>
    </>
  );
}

function DiffRenderer({ n, onAnswer }) {
  useFocusKeys([
    { match: (e) => (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) || e.key.toLowerCase() === 'a',
      fn: () => onAnswer('Approved') },
    { match: (e) => e.key.toLowerCase() === 'r',
      fn: () => onAnswer('Rejected') },
    { match: (e) => e.key.toLowerCase() === 's',
      fn: () => onAnswer('Snoozed 1h') },
  ], [n, onAnswer]);
  return (
    <>
      <div className="diff">
        {n.diff.map((l, i) => (
          <div key={i} className={`line ${l.kind}`}>{l.text}</div>
        ))}
      </div>
      <div className="action-row">
        <button className="btn-action danger" onClick={() => onAnswer('Rejected')}>
          Reject <span className="k">R</span>
        </button>
        <button className="btn-action ghost" onClick={() => onAnswer('Snoozed 1h')}>
          Snooze 1h <span className="k">S</span>
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
    { match: (e) => e.key.toLowerCase() === 'j' && !document.activeElement?.matches('input,textarea'),
      fn: () => setCursor((c) => Math.min(items.length - 1, c + 1)) },
    { match: (e) => e.key.toLowerCase() === 'k' && !document.activeElement?.matches('input,textarea'),
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
          placeholder="Search hotels…"
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
          Book selection <span className="k">⏎</span>
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
          Book this slot <span className="k">⏎</span>
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

// ── Focus shell ─────────────────────────────────────────────────────────

function Focus({ n, onClose, onAnswer }) {
  const agent = window.AGENTS[n.agent];
  const Renderer = RENDERERS[n.type];
  const agentColor = window.agentColor(n.agent);

  React.useEffect(() => {
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <>
      <div className="focus-backdrop" onClick={onClose} />
      <div className="focus" style={{ '--agent': agentColor }}>
        <div className="focus-hd">
          <div className="glyph">{agent.glyph}</div>
          <div className="meta">
            <div className="label">
              <strong>{n.sender}</strong> via {agent.name} · {fmtSentAt(n.sentAt)} · {TYPE_LABEL[n.type]}
            </div>
            <div className="sender" style={{ marginTop: 2 }}>
              Urgency {n.urgency}/5 · {n.blocking ? 'Blocking' : 'Non-blocking'}
            </div>
          </div>
          {n.deadlineMs && <Countdown ms={n.deadlineMs} urgent={n.urgency >= 4} />}
          <button className="close" onClick={onClose}>✕</button>
        </div>
        <div className="focus-body">
          <h2 className="focus-q">{n.question}</h2>
          {n.detail && <p className="focus-detail">{n.detail}</p>}
          <Renderer n={n} onAnswer={onAnswer} />
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
