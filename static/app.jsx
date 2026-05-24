// SJBIS — Main app shell (production version).
// Dashboard: top command bar + left agent rail + main field of cards + history.
// Connects to real daemon via REST + SSE.

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "motionIntensity": 7,
  "palette": "lime",
  "showConnections": true,
  "compactRail": false,
  "textRail": true,
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

// Global API helpers
const API_BASE = window.location.origin;

async function apiState() {
  const r = await fetch(`${API_BASE}/state`);
  if (!r.ok) throw new Error('failed to load state');
  return r.json();
}

async function apiAnswer(id, answer, via = 'dashboard', note = null) {
  const body = { answer, via };
  if (note) body.note = note;
  const r = await fetch(`${API_BASE}/answer/${id}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error('failed to post answer');
  return r.json();
}

async function apiAddRule(text, scope, urgencyMin, mute) {
  const r = await fetch(`${API_BASE}/rules`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ text, scope, urgency_min: urgencyMin, mute }),
  });
  if (!r.ok) throw new Error('failed to add rule');
  return r.json();
}

async function apiSnooze(id, minutes) {
  const r = await fetch(`${API_BASE}/snooze/${id}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ minutes }),
  });
  if (!r.ok) {
    const err = await r.json().catch(() => ({}));
    throw new Error(err.error || `failed to snooze (${r.status})`);
  }
  return r.json();
}

async function apiDismiss(id) {
  const r = await fetch(`${API_BASE}/dismiss/${id}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
  });
  if (!r.ok) {
    const err = await r.json().catch(() => ({}));
    throw new Error(err.error || `failed to dismiss (${r.status})`);
  }
  return r.json();
}

async function apiDeleteRule(id) {
  await fetch(`${API_BASE}/rules/${id}`, { method: 'DELETE' });
}

function NotificationCard({ n, onClick, onDismiss, agents, selected, cardRef }) {
  const agent = agents[n.agent_name] || agents[n.agent] || { glyph: '◐', name: n.agent_name || n.agent };
  const color = window.agentColor(n.agent_name || n.agent);
  return (
    <div
      ref={cardRef}
      className={`card u${n.urgency}${selected ? ' is-selected' : ''}`}
      style={{ '--agent': color, animationDelay: `${0.04 * Math.random()}s` }}
      onClick={onClick}
    >
      <div className="ribbon" />
      <span className="kbd-marker">↵ open</span>
      <button
        className="card-dismiss"
        title="Dismiss without reply (d)"
        onClick={(e) => { e.stopPropagation(); onDismiss(n.id); }}
      >
        <span className="k">d</span>
      </button>
      <div className="card-hd">
        <div className="glyph">{agent.glyph}</div>
        <div className="meta">
          <div className="name">{agent.name} · {n.sender || n.instance || 'unknown'}</div>
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
      <div className="card-meta">
        {n.src && (
          <div className="src" title={`SRC: ${n.src}`}>
            <b>SRC</b>{n.src}
          </div>
        )}
      </div>
      <div className="card-ft">
        <div className="type-pill">
          <span className="ic">{TYPE_ICONS[n.question_type] || TYPE_ICONS[n.type] || '·'}</span>
          {window.TYPE_LABEL[n.question_type || n.type] || n.question_type || n.type}
        </div>
        <div className="deadline">
          {n.deadline ? `⏱ ${Math.floor((new Date(n.deadline) - Date.now()) / 60000)}m` : window.fmtSentAt(n.sentAt || n.created_at)}
        </div>
      </div>
    </div>
  );
}

function AgentRail({ agents, counts, filterAgent, onFilterAgent, textMode }) {
  return (
    <div className={'rail' + (textMode ? ' text-mode' : '')}>
      <div className="lbl">SRC</div>
      {Object.entries(agents).map(([id, a]) => (
        <div
          key={id}
          className={'agent-pill' + (filterAgent === id ? ' active' : '') + (textMode ? ' text' : '')}
          style={{ borderColor: counts[id] ? window.agentColor(id) : undefined }}
          title={a.name}
          onClick={() => onFilterAgent(id)}
        >
          {textMode ? (
            <span className="agent-name" style={{ color: window.agentColor(id) }}>{a.name}</span>
          ) : (
            <span style={{ color: window.agentColor(id) }}>{a.glyph}</span>
          )}
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

function formatRuleExpiry(expiresAt) {
  if (!expiresAt) return null;
  const exp = new Date(expiresAt);
  const now = new Date();
  const diffMs = exp - now;
  if (diffMs <= 0) return 'expired';
  const mins = Math.floor(diffMs / 60000);
  if (mins < 60) return `${mins}m left`;
  const hours = Math.floor(mins / 60);
  return `${hours}h left`;
}

function CommandBar({ onAddRule }) {
  const [input, setInput] = React.useState('');
  const [rules, setRules] = React.useState([]);

  const submit = () => {
    if (!input.trim()) return;
    const text = input.trim();
    const isMute = /mute|silence|quiet|hide/i.test(text);
    const isAllow = /^allow /i.test(text) || /^only allow /i.test(text);
    const rule = { id: 'r' + Date.now(), text, active: true, scope: 'inbox-agent', urgencyMin: 0, mute: isMute };
    setRules([...rules, rule]);
    // For allow-list patterns, the server creates multiple rules
    onAddRule(text, 'inbox-agent', 0, isMute).then((created) => {
      if (Array.isArray(created)) {
        // Server returned multiple rules
        const newRules = created.map((r) => ({ ...r, active: true }));
        setRules((prev) => [...newRules, ...prev.filter((p) => !newRules.find((n) => n.id === p.id)))]);
      } else {
        setRules((prev) => [created, ...prev.filter((p) => p.id !== created.id)]);
      }
    }).catch(console.error);
    setInput('');
  };
  const remove = (id) => {
    setRules(rules.filter((r) => r.id !== id));
    apiDeleteRule(id).catch(console.error);
  };
  return (
    <div className="cmd">
      <span className="slash">RULE /</span>
      <input
        value={input}
        placeholder='e.g. "allow iMessage from Jeff, Carmen for 1h" or "mute all iMessage"'
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => e.key === 'Enter' && submit()}
      />
      <div className="chips">
        {rules.slice(0, 6).map((r) => {
          const expiry = formatRuleExpiry(r.expires_at || r.expiresAt);
          const pri = r.priority > 0 ? `[${r.priority}] ` : '';
          return (
            <span key={r.id} className={'chip' + (r.mute ? ' mute' : '') + (expiry === 'expired' ? ' expired' : '')}>
              <span className="x" onClick={() => remove(r.id)}>×</span>
              {pri}{r.text.length > 32 ? r.text.slice(0, 30) + '…' : r.text}
              {expiry && <span className="chip-expiry">{expiry}</span>}
            </span>
          );
        })}
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
        <div key={h.id} className="h-item" style={{ '--agent': window.agentColor(h.agent_name || h.agent) }}>
          <div className="h-top">
            <span className="dotc" />
            <span>{(window.AGENTS && window.AGENTS[h.agent_name || h.agent]?.name) || h.agent_name || h.agent}</span>
            <span style={{ marginLeft: 'auto' }}>{h.answered_at ? new Date(h.answered_at).toLocaleTimeString() : 'just now'}</span>
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
  const [notifications, setNotifications] = React.useState([]);
  const [history, setHistory] = React.useState([]);
  const [rules, setRules] = React.useState([]);
  const [agents, setAgents] = React.useState({});
  const [filterAgent, setFilterAgent] = React.useState(null);
  const [focused, setFocused] = React.useState(null);
  const [burst, setBurst] = React.useState(null);
  const [selectedIdx, setSelectedIdx] = React.useState(0);
  const [connected, setConnected] = React.useState(false);
  const cardRefs = React.useRef({});

  // Parse ?q_id=... from URL for deep-linking to a notification
  const urlQId = React.useMemo(() => {
    const params = new URLSearchParams(window.location.search);
    return params.get('q_id');
  }, []);

  // Load initial state
  React.useEffect(() => {
    apiState()
      .then((state) => {
        setNotifications(state.notifications || []);
        setHistory(state.history || []);
        setRules(state.rules || []);
        setAgents(state.agents || {});
        window.AGENTS = state.agents || {};

        // Deep-link: if q_id is in URL, focus that notification
        if (urlQId) {
          const n = (state.notifications || []).find((x) => x.id === urlQId);
          if (n) setFocused(n);
        }
      })
      .catch((e) => console.error('Failed to load state:', e));
  }, [urlQId]);

  // Sync focused notification to URL (replaceState so back button closes it)
  React.useEffect(() => {
    const url = new URL(window.location);
    if (focused) {
      url.searchParams.set('q_id', focused.id);
    } else {
      url.searchParams.delete('q_id');
    }
    window.history.replaceState({}, '', url);
  }, [focused]);

  // SSE connection
  React.useEffect(() => {
    const source = new EventSource(`${API_BASE}/events`);
    source.onopen = () => setConnected(true);
    source.onerror = () => setConnected(false);
    source.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        if (!event.event) return;
        switch (event.event) {
          case 'notification_created':
            setNotifications((prev) => [event.notification, ...prev]);
            break;
          case 'notification_updated': {
            const now = Date.now();
            const snoozeUntil = event.notification?.snooze_until ? new Date(event.notification.snooze_until).getTime() : 0;
            if (snoozeUntil > now) {
              // Notification was snoozed — remove it from the active list
              setNotifications((prev) => prev.filter((n) => n.id !== event.notification.id));
              // Also clear focus if this was the focused notification
              setFocused((f) => (f && f.id === event.notification.id) ? null : f);
            } else {
              setNotifications((prev) => prev.map((n) => n.id === event.notification.id ? event.notification : n));
            }
            break;
          }
          case 'notification_answered':
            setNotifications((prev) => prev.filter((n) => n.id !== event.envelope.id));
            // Add to history
            const answered = event.envelope;
            setHistory((prev) => [
              { id: answered.id, agent_name: answered.src, question: answered.question || '', answer: answered.answer, answered_at: answered.answered_at, type: answered.renderer },
              ...prev,
            ]);
            break;
          case 'notification_cancelled':
            setNotifications((prev) => prev.filter((n) => n.id !== event.id));
            break;
          case 'notification_dismissed':
            setNotifications((prev) => prev.filter((n) => n.id !== event.id));
            setFocused((f) => (f && f.id === event.id) ? null : f);
            break;
          case 'rule_created':
            setRules((prev) => [event.rule, ...prev]);
            break;
          case 'rule_deleted':
            setRules((prev) => prev.filter((r) => r.id !== event.id));
            break;
        }
      } catch (err) {
        console.error('SSE parse error:', err);
      }
    };
    return () => source.close();
  }, []);

  // Periodic background sync: catches out-of-band deletions (e.g. DB cleared)
  React.useEffect(() => {
    const sync = () => {
      apiState()
        .then((state) => {
          setNotifications((prev) => {
            const serverIds = new Set((state.notifications || []).map((n) => n.id));
            return prev.filter((n) => serverIds.has(n.id));
          });
          setHistory(state.history || []);
          setRules(state.rules || []);
          setAgents(state.agents || {});
          window.AGENTS = state.agents || {};
        })
        .catch((e) => console.error('Background sync failed:', e));
    };
    const id = setInterval(sync, 30000);
    return () => clearInterval(id);
  }, []);

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

  // Motion intensity
  React.useEffect(() => {
    const k = (t.motionIntensity ?? 7) / 7;
    document.documentElement.style.setProperty('--motion-k', k);
  }, [t.motionIntensity]);

  // Counts per agent for rail badges
  const counts = React.useMemo(() => {
    const c = {};
    for (const a of Object.keys(agents)) c[a] = 0;
    for (const n of notifications) {
      const key = n.agent_name || n.agent;
      c[key] = (c[key] || 0) + 1;
    }
    return c;
  }, [notifications, agents]);

  // Filtered to single agent when rail is clicked
  const visible = React.useMemo(
    () => notifications
      .filter((n) => !filterAgent || (n.agent_name || n.agent) === filterAgent)
      .sort((a, b) => b.urgency - a.urgency),
    [notifications, filterAgent]
  );

  // Keep selection in bounds
  React.useEffect(() => {
    if (visible.length === 0) return;
    if (selectedIdx >= visible.length) setSelectedIdx(visible.length - 1);
  }, [visible.length, selectedIdx]);

  // Dashboard-level keyboard nav
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
      } else if (key === 't') {
        e.preventDefault();
        window.postMessage({ type: '__activate_edit_mode' }, '*');
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [visible, selectedIdx, focused]);

  // Scroll selected card into view
  React.useEffect(() => {
    const n = visible[selectedIdx];
    if (!n) return;
    const el = cardRefs.current[n.id];
    if (el && typeof el.scrollIntoView === 'function') {
      const canvas = el.closest('.canvas');
      if (canvas) {
        const er = el.getBoundingClientRect();
        const cr = canvas.getBoundingClientRect();
        if (er.top < cr.top + 20) canvas.scrollTop += er.top - cr.top - 20;
        else if (er.bottom > cr.bottom - 20) canvas.scrollTop += er.bottom - cr.bottom + 20;
      }
    }
  }, [selectedIdx, visible]);

  const onAnswer = async (val, note = null) => {
    const n = focused;
    if (!n) return;
    const isSkip = val === '(skipped)';
    try {
      if (!isSkip) {
        await apiAnswer(n.id, typeof val === 'string' ? val : String(val), 'dashboard', note);
      }
      setNotifications((prev) => prev.filter((x) => x.id !== n.id));
      if (!isSkip) {
        setHistory((prev) => [
          { id: n.id, agent_name: n.agent_name || n.agent, question: n.question,
            answer: typeof val === 'string' ? val : String(val), type: n.question_type || n.type,
            answeredAt: new Date().toISOString(), note },
          ...prev,
        ]);
      }
      setFocused(null);
      if (!isSkip) {
        setBurst({
          text: (n.question_type || n.type) === 'ack' ? 'noted' : 'sent',
          color: 'var(--lime)',
        });
      }
    } catch (e) {
      console.error('Failed to answer:', e);
    }
  };

  const onDismiss = async () => {
    const n = focused;
    if (!n) return;
    try {
      await apiDismiss(n.id);
      setNotifications((prev) => prev.filter((x) => x.id !== n.id));
      setFocused(null);
      setBurst({ text: 'dismissed', color: 'var(--ink-3)' });
    } catch (e) {
      console.error('Failed to dismiss:', e);
    }
  };

  const onFilterAgent = (id) => {
    setFilterAgent((current) => current === id ? null : id);
    setSelectedIdx(0);
  };

  // Demo notifications are no longer injected client-side.
  // Use the TweaksPanel "Trigger urgent notification" button or `sjbis ask` CLI.

  return (
    <>
      <div className="field" />
      <div className={'app' + (t.textRail ? ' text-rail' : '')}>
        <div className="topbar">
          <div className="brand">
            <span className="dot" />
            <span>sjbis</span>
            <span className="sub">information surfacer · v0.1{connected ? '' : ' · offline'}</span>
          </div>
          <CommandBar onAddRule={apiAddRule} />
          <LiveClock />
          <button
            className="settings-btn"
            title="Settings (T)"
            onClick={() => window.postMessage({ type: '__activate_edit_mode' }, '*')}
          >
            ⚙
          </button>
        </div>

        <AgentRail
          agents={agents}
          counts={counts}
          filterAgent={filterAgent}
          onFilterAgent={onFilterAgent}
          textMode={t.textRail}
        />

        <div className="canvas">
          <div className="canvas-hd">
            <h1>Awaiting your attention</h1>
            <span className="count">
              <strong>{visible.length}</strong> open ·{' '}
              {visible.filter((v) => v.urgency >= 4).length} urgent
              {filterAgent && (
                <>{' · Showing only: '}{agents[filterAgent]?.name || filterAgent}</>
              )}
            </span>
          </div>
          <div className="field-grid">
            {visible.map((n, i) => (
              <NotificationCard
                key={n.id}
                n={n}
                agents={agents}
                selected={i === selectedIdx}
                cardRef={(el) => { cardRefs.current[n.id] = el; }}
                onClick={() => { setSelectedIdx(i); setFocused(n); }}
                onDismiss={async (id) => {
                  try {
                    await apiDismiss(id);
                    setNotifications((prev) => prev.filter((x) => x.id !== id));
                  } catch (e) {
                    console.error('Failed to dismiss:', e);
                  }
                }}
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
        <window.Focus n={focused} onClose={() => setFocused(null)} onAnswer={onAnswer} onDismiss={onDismiss} onSnooze={(minutes) => apiSnooze(focused.id, minutes).then(() => { setFocused(null); }).catch((e) => { console.error('Snooze failed:', e); alert(e.message); })} />
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
      {focused && (
        <div className="kbd-help" aria-hidden="true">
          <span className="grp"><kbd>d</kbd> dismiss</span>
          <span className="grp"><kbd>s</kbd> snooze</span>
          <span className="grp"><kbd>⇧N</kbd> note</span>
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
          label="Text rail (monospace names instead of icons)" value={t.textRail}
          onChange={(v) => setTweak('textRail', v)}
        />
        <window.TweakToggle
          label="Show connection lines" value={t.showConnections}
          onChange={(v) => setTweak('showConnections', v)}
        />
        <window.TweakSection label="Demo" />
        <window.TweakButton
          label="Trigger urgent notification"
          onClick={async () => {
            const fresh = {
              question: 'Approve $4,221 wire to "Vendor Solutions LLC"?',
              agent_name: 'guard',
              urgency: 5,
              yesno: true,
              detail: 'New payee. Bank flagged as unusual. Replies needed within 90s.',
            };
            await fetch(`${API_BASE}/ask`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify(fresh),
            });
          }}
        />
      </window.TweaksPanel>
    </>
  );
}

ReactDOM.createRoot(document.getElementById('root')).render(<App />);
