// SJBIS — Main app shell.
// Dashboard: top command bar + left agent rail + main field of cards + history.
// Focus mode: click a card to enter; per-type renderer in focus.jsx handles the body.

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "motionIntensity": 7,
  "palette": "lime",
  "showConnections": true,
  "compactRail": false,
  "incomingDemo": true
}/*EDITMODE-END*/;

const PALETTES = {
  lime:    { primary: '#C7F33D', hot: '#FF3D7F', siren: '#FF1F4D', warm: '#FFB341', calm: '#5BD4FF', violet: '#B89DFF' },
  citrus:  { primary: '#FFB341', hot: '#FF6B3D', siren: '#FF3D1F', warm: '#FFE066', calm: '#5BD4FF', violet: '#B89DFF' },
  electric:{ primary: '#5BD4FF', hot: '#FF3D7F', siren: '#FF1F4D', warm: '#FFB341', calm: '#7AE7FF', violet: '#B89DFF' },
  magenta: { primary: '#FF3D7F', hot: '#FF6BA8', siren: '#FF1F4D', warm: '#FFB341', calm: '#5BD4FF', violet: '#B89DFF' },
};

// Type pill icons — tiny SVG glyphs
const TYPE_ICONS = {
  yesno:       '⊕',
  multichoice: '◉',
  freetext:    '¶',
  numeric:     '#',
  file:        '↥',
  diff:        '±',
  ack:         '✓',
  picklist:    '☰',
  schedule:    '◷',
};

function NotificationCard({ n, onClick, agents, selected, cardRef }) {
  const agent = agents[n.agent];
  const color = window.agentColor(n.agent);
  return (
    <div
      ref={cardRef}
      className={`card u${n.urgency}${selected ? ' is-selected' : ''}`}
      style={{ '--agent': color, animationDelay: `${0.04 * Math.random()}s` }}
      onClick={onClick}
    >
      <div className="ribbon" />
      <span className="kbd-marker">↵ open</span>
      <div className="card-hd">
        <div className="glyph">{agent.glyph}</div>
        <div className="meta">
          <div className="name">{agent.name} · {n.sender}</div>
        </div>
        <div className={`urgency urg-${n.urgency}`}>
          {n.urgency >= 4 ? 'URGENT' : n.urgency === 3 ? 'TIMELY' : 'CALM'}
        </div>
      </div>
      <div className="q">{n.question}</div>
      {n.choices && (
        <div className="preview-row">
          {n.choices.slice(0, 3).map((c) => (
            <span key={c.value} className="preview-chip">{c.label}</span>
          ))}
          {n.choices.length > 3 && <span className="preview-chip">+{n.choices.length - 3}</span>}
        </div>
      )}
      <div className="type-pill">
        <span className="ic">{TYPE_ICONS[n.type] || '·'}</span>
        {window.TYPE_LABEL[n.type]}
      </div>
      {n.src && (
        <div className="src" title={`SRC: ${n.src}`}>
          <b>SRC:</b>{n.src}
        </div>
      )}
      <div className="deadline">
        {n.deadlineMs ? `⏱ ${Math.floor(n.deadlineMs / 60000)}m` : window.fmtSentAt(n.sentAt)}
      </div>
    </div>
  );
}

function AgentRail({ agents, counts, muted, onToggleMute }) {
  return (
    <div className="rail">
      <div className="lbl">SRC</div>
      {Object.entries(agents).map(([id, a]) => (
        <div
          key={id}
          className={'agent-pill' + (muted.has(id) ? ' muted' : '')}
          style={{ borderColor: counts[id] ? window.agentColor(id) : undefined }}
          title={a.name}
          onClick={() => onToggleMute(id)}
        >
          <span style={{ color: window.agentColor(id) }}>{a.glyph}</span>
          {counts[id] > 0 && (
            <span className="badge" style={{ background: window.agentColor(id) }}>
              {counts[id]}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

function CommandBar({ rules, setRules }) {
  const [input, setInput] = React.useState('');
  const submit = () => {
    if (!input.trim()) return;
    const text = input.trim();
    const isMute = /mute|silence|quiet|hide/i.test(text);
    setRules([
      ...rules,
      { id: 'r' + Date.now(), text, active: true, scope: 'inbox-agent', urgencyMin: 0, mute: isMute },
    ]);
    setInput('');
  };
  const remove = (id) => setRules(rules.filter((r) => r.id !== id));
  return (
    <div className="cmd">
      <span className="slash">RULE /</span>
      <input
        value={input}
        placeholder='Tell me what to surface — e.g. "only family + code agents for the next hour"'
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => e.key === 'Enter' && submit()}
      />
      <div className="chips">
        {rules.slice(0, 4).map((r) => (
          <span key={r.id} className={'chip' + (r.mute ? ' mute' : '')}>
            <span className="x" onClick={() => remove(r.id)}>×</span>
            {r.text.length > 32 ? r.text.slice(0, 30) + '…' : r.text}
          </span>
        ))}
      </div>
    </div>
  );
}

function History({ items, onReplay }) {
  return (
    <div className="history">
      <div className="history-hd">
        <span>Recent · answered</span>
        <span className="replay" onClick={onReplay}>↻ replay</span>
      </div>
      {items.map((h) => (
        <div key={h.id} className="h-item" style={{ '--agent': window.agentColor(h.agent) }}>
          <div className="h-top">
            <span className="dotc" />
            <span>{window.AGENTS[h.agent].name}</span>
            <span style={{ marginLeft: 'auto' }}>{h.answeredAt}</span>
          </div>
          <div className="h-q">{h.question}</div>
          <div className="h-a">
            {h.answer}
            {h.answer2 && <span style={{ color: 'var(--ink-4)', marginLeft: 4 }}>{h.answer2}</span>}
          </div>
        </div>
      ))}
    </div>
  );
}

// Live clock
function LiveClock() {
  const [now, setNow] = React.useState(new Date());
  React.useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(t);
  }, []);
  const hh = String(now.getHours()).padStart(2, '0');
  const mm = String(now.getMinutes()).padStart(2, '0');
  const ss = String(now.getSeconds()).padStart(2, '0');
  return (
    <div className="clock">
      <span className="live">LIVE</span>
      <span>{hh}:{mm}:{ss}</span>
    </div>
  );
}

// ── App ────────────────────────────────────────────────────────────────

function App() {
  const [t, setTweak] = window.useTweaks(TWEAK_DEFAULTS);
  const [notifications, setNotifications] = React.useState(window.SEED_NOTIFICATIONS);
  const [history, setHistory] = React.useState(window.HISTORY);
  const [rules, setRules] = React.useState(window.SEED_RULES);
  const [muted, setMuted] = React.useState(new Set());
  const [focused, setFocused] = React.useState(null);
  const [burst, setBurst] = React.useState(null);
  const [selectedIdx, setSelectedIdx] = React.useState(0);
  const cardRefs = React.useRef({});
  const incomingTriggered = React.useRef(false);

  // Override palette CSS vars from tweak
  React.useEffect(() => {
    const p = PALETTES[t.palette] || PALETTES.lime;
    const root = document.documentElement;
    root.style.setProperty('--lime', p.primary);
    root.style.setProperty('--hot', p.hot);
    root.style.setProperty('--siren', p.siren);
    root.style.setProperty('--warm', p.warm);
    root.style.setProperty('--calm', p.calm);
    root.style.setProperty('--violet', p.violet);
  }, [t.palette]);

  // Motion intensity: tone down/up the animations globally
  React.useEffect(() => {
    const k = (t.motionIntensity ?? 7) / 7;
    document.documentElement.style.setProperty('--motion-k', k);
  }, [t.motionIntensity]);

  // Counts per agent for rail badges
  const counts = React.useMemo(() => {
    const c = {};
    for (const a of Object.keys(window.AGENTS)) c[a] = 0;
    for (const n of notifications) c[n.agent] = (c[n.agent] || 0) + 1;
    return c;
  }, [notifications]);

  // Filtered by mute set
  const visible = React.useMemo(
    () => notifications.filter((n) => !muted.has(n.agent))
      .sort((a, b) => b.urgency - a.urgency),
    [notifications, muted]
  );

  // Keep selection in bounds when the visible set changes (e.g. answer removes a card)
  React.useEffect(() => {
    if (visible.length === 0) return;
    if (selectedIdx >= visible.length) setSelectedIdx(visible.length - 1);
  }, [visible.length, selectedIdx]);

  // Dashboard-level keyboard nav. Suppress when typing in a control or in focus
  // mode (focus renderers own their own keys).
  React.useEffect(() => {
    const isTyping = (el) => {
      if (!el) return false;
      const tag = el.tagName;
      return tag === 'INPUT' || tag === 'TEXTAREA' || el.isContentEditable;
    };
    const onKey = (e) => {
      if (focused) return;
      if (isTyping(e.target)) return;
      if (visible.length === 0) return;
      const key = e.key.toLowerCase();
      if (key === 'j' || e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIdx((i) => Math.min(visible.length - 1, i + 1));
      } else if (key === 'k' || e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIdx((i) => Math.max(0, i - 1));
      } else if (key === 'g') {
        e.preventDefault();
        setSelectedIdx(0);
      } else if (e.key === 'G') {
        e.preventDefault();
        setSelectedIdx(visible.length - 1);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const n = visible[selectedIdx];
        if (n) setFocused(n);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [visible, selectedIdx, focused]);

  // Scroll the selected card into view as the user navigates
  React.useEffect(() => {
    const n = visible[selectedIdx];
    if (!n) return;
    const el = cardRefs.current[n.id];
    if (el && typeof el.scrollIntoView === 'function') {
      // Avoid scrollIntoView per project rules — manual scroll on the canvas
      const canvas = el.closest('.canvas');
      if (canvas) {
        const er = el.getBoundingClientRect();
        const cr = canvas.getBoundingClientRect();
        if (er.top < cr.top + 20) canvas.scrollTop += er.top - cr.top - 20;
        else if (er.bottom > cr.bottom - 20) canvas.scrollTop += er.bottom - cr.bottom + 20;
      }
    }
  }, [selectedIdx, visible]);

  const onAnswer = (val) => {
    const n = focused;
    if (!n) return;
    setNotifications((prev) => prev.filter((x) => x.id !== n.id));
    setHistory((prev) => [
      { id: 'h-' + Date.now(), agent: n.agent, question: n.question,
        answer: typeof val === 'string' ? val : String(val), type: n.type,
        answeredAt: 'just now' },
      ...prev,
    ]);
    setFocused(null);
    setBurst({
      text: n.type === 'ack' ? 'noted' : 'sent',
      color: 'var(--lime)',
    });
  };

  const onToggleMute = (id) => {
    setMuted((m) => {
      const n = new Set(m);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });
  };

  // Demo: incoming notification flash after 6 seconds
  React.useEffect(() => {
    if (!t.incomingDemo || incomingTriggered.current) return;
    const tm = setTimeout(() => {
      incomingTriggered.current = true;
      const fresh = {
        id: 'n-incoming',
        agent: 'fam',
        sender: 'Mia',
        type: 'yesno',
        urgency: 5,
        blocking: true,
        sentAt: '-00:00:01',
        deadlineMs: 4 * 60 * 1000,
        question: 'Can I have a sleepover at Aya\'s tonight?',
        detail: 'Mia just texted. Aya\'s parents (Lin & Rob) confirmed, and Mia would be picked up 9am Sunday.',
        yesLabel: 'Yes, ok',
        noLabel: 'Not tonight',
      };
      setNotifications((prev) => [fresh, ...prev]);
    }, 6000);
    return () => clearTimeout(tm);
  }, [t.incomingDemo]);

  return (
    <>
      <div className="field" />
      <div className="app">
        <div className="topbar">
          <div className="brand">
            <span className="dot" />
            <span>sjbis</span>
            <span className="sub">information surfacer · v0.4</span>
          </div>
          <CommandBar rules={rules} setRules={setRules} />
          <LiveClock />
        </div>

        <AgentRail
          agents={window.AGENTS}
          counts={counts}
          muted={muted}
          onToggleMute={onToggleMute}
        />

        <div className="canvas">
          <div className="canvas-hd">
            <h1>Awaiting your attention</h1>
            <span className="count">
              <strong>{visible.length}</strong> open ·{' '}
              {visible.filter((v) => v.urgency >= 4).length} urgent ·{' '}
              {muted.size} source{muted.size !== 1 ? 's' : ''} muted
            </span>
          </div>
          <div className="field-grid">
            {visible.map((n, i) => (
              <NotificationCard
                key={n.id}
                n={n}
                agents={window.AGENTS}
                selected={i === selectedIdx}
                cardRef={(el) => { cardRefs.current[n.id] = el; }}
                onClick={() => { setSelectedIdx(i); setFocused(n); }}
              />
            ))}
            {visible.length === 0 && (
              <div style={{
                gridColumn: '1/-1',
                padding: '60px',
                textAlign: 'center',
                color: 'var(--ink-3)',
                fontFamily: 'var(--display)',
                fontSize: 22,
              }}>
                All clear. Nothing is asking for your attention.
              </div>
            )}
          </div>
        </div>

        <History items={history} onReplay={() => setBurst({ text: 'replay queued', color: 'var(--calm)' })} />
      </div>

      {focused && (
        <window.Focus n={focused} onClose={() => setFocused(null)} onAnswer={onAnswer} />
      )}
      {burst && <window.Burst text={burst.text} color={burst.color} onDone={() => setBurst(null)} />}

      {!focused && visible.length > 0 && (
        <div className="kbd-help" aria-hidden="true">
          <span className="grp"><kbd>J</kbd><kbd>K</kbd> navigate</span>
          <span className="grp"><kbd>↵</kbd> open</span>
          <span className="grp"><kbd>1</kbd>–<kbd>9</kbd> answer</span>
          <span className="grp"><kbd>esc</kbd> back</span>
        </div>
      )}

      <window.TweaksPanel title="SJBIS tweaks">
        <window.TweakSection label="Vibe" />
        <window.TweakSlider
          label="Motion intensity" value={t.motionIntensity} min={0} max={10} step={1}
          onChange={(v) => setTweak('motionIntensity', v)}
        />
        <window.TweakSelect
          label="Palette"
          value={t.palette}
          options={[
            { value: 'lime',     label: 'Lime · acid' },
            { value: 'citrus',   label: 'Citrus · warm' },
            { value: 'electric', label: 'Electric · cyan' },
            { value: 'magenta',  label: 'Magenta · hot' },
          ]}
          onChange={(v) => setTweak('palette', v)}
        />
        <window.TweakSection label="Layout" />
        <window.TweakToggle
          label="Compact agent rail" value={t.compactRail}
          onChange={(v) => setTweak('compactRail', v)}
        />
        <window.TweakToggle
          label="Show connection lines" value={t.showConnections}
          onChange={(v) => setTweak('showConnections', v)}
        />
        <window.TweakSection label="Demo" />
        <window.TweakToggle
          label="Auto-incoming flash" value={t.incomingDemo}
          onChange={(v) => setTweak('incomingDemo', v)}
        />
        <window.TweakButton
          label="Trigger urgent notification"
          onClick={() => {
            const fresh = {
              id: 'n-' + Date.now(),
              agent: 'guard',
              sender: 'Sentinel',
              type: 'yesno',
              urgency: 5,
              blocking: true,
              sentAt: '-00:00:01',
              deadlineMs: 90 * 1000,
              question: 'Approve $4,221 wire to "Vendor Solutions LLC"?',
              detail: 'New payee. Bank flagged as unusual. Replies needed within 90s.',
              yesLabel: 'Approve wire',
              noLabel: 'Block & alert',
            };
            setNotifications((prev) => [fresh, ...prev]);
          }}
        />
      </window.TweaksPanel>
    </>
  );
}

ReactDOM.createRoot(document.getElementById('root')).render(<App />);
