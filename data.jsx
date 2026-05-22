// Mock notifications + agent registry for SJBIS.
// Each notification is what some external tool would have shipped to the
// surfacer via the (hypothetical) CLI invocation.

// Deterministic palette per agent id — saturated OKLCH so identities
// stay visually distinct on the dark field.
function agentColor(id) {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) % 360;
  return `oklch(78% 0.18 ${h})`;
}
function agentColorDim(id) {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) % 360;
  return `oklch(58% 0.14 ${h})`;
}

const AGENTS = {
  'inbox-agent': { name: 'Inbox',    glyph: '◐', kind: 'email' },
  'cal-agent':   { name: 'Calendar', glyph: '◧', kind: 'schedule' },
  'code-agent':  { name: 'Coder',    glyph: '⌬', kind: 'code' },
  'pay-agent':   { name: 'Ledger',   glyph: '$',  kind: 'finance' },
  'fam':         { name: 'Family',   glyph: '♡', kind: 'people' },
  'shop-agent':  { name: 'Shopper',  glyph: '☁', kind: 'commerce' },
  'doc-agent':   { name: 'Docs',     glyph: '¶', kind: 'docs' },
  'guard':       { name: 'Sentinel', glyph: '⌖', kind: 'security' },
  'tax-agent':   { name: 'Accountant', glyph: '∑', kind: 'finance' },
  'travel':      { name: 'Travel',   glyph: '✈', kind: 'travel' },
};

// 9 question types, each with a representative example:
//  yesno · multichoice · freetext · numeric · file · diff · ack · picklist · schedule
const SEED_NOTIFICATIONS = [
  {
    id: 'n-001',
    agent: 'fam',
    sender: 'Joey',
    src: 'iMessage via Hermes',
    type: 'multichoice',
    urgency: 4,
    blocking: false,
    sentAt: '-00:00:18',
    deadlineMs: 8 * 60 * 1000,
    question: 'Clam chowder vote — they\'re ordering in 10.',
    detail: 'Joey is at Legal Seafoods with the cousins. Needs your pick before the server comes back.',
    choices: [
      { value: 'new-england', label: 'New England', hint: 'White, cream' },
      { value: 'manhattan',   label: 'Manhattan',   hint: 'Red, tomato' },
      { value: 'rhode-island', label: 'Rhode Island', hint: 'Clear broth' },
      { value: 'skip',         label: 'Skip soup',   hint: 'Just the entrée' },
    ],
  },
  {
    id: 'n-002',
    agent: 'tax-agent',
    sender: 'Dana, CPA',
    src: 'Gmail via Postmaster',
    type: 'yesno',
    urgency: 5,
    blocking: true,
    sentAt: '-00:01:42',
    deadlineMs: 6 * 60 * 1000,
    question: 'Claim the Tuesday burrito as a business expense?',
    detail: 'Receipt for $14.20 at Cilantro. Dana flagged it because the meeting with Priya was on calendar — looks deductible.',
    yesLabel: 'Yes, claim it',
    noLabel: 'No, personal',
  },
  {
    id: 'n-003',
    agent: 'code-agent',
    sender: 'Migrator',
    src: 'OpenCode Session s7b3d11',
    type: 'diff',
    urgency: 3,
    blocking: false,
    sentAt: '-00:04:11',
    question: 'Approve renaming `getUser` → `fetchUser` across 14 files?',
    detail: 'Codemod ran clean. 23 callsites updated, 2 tests changed names. No behavior diff in the test run.',
    diff: [
      { kind: 'meta', text: 'src/api/users.ts' },
      { kind: 'del',  text: '-export function getUser(id: string) {' },
      { kind: 'add',  text: '+export function fetchUser(id: string) {' },
      { kind: 'ctx',  text: '   return db.users.findById(id);' },
      { kind: 'ctx',  text: ' }' },
      { kind: 'meta', text: 'src/pages/profile.tsx' },
      { kind: 'del',  text: '-import { getUser } from "@/api/users";' },
      { kind: 'add',  text: '+import { fetchUser } from "@/api/users";' },
      { kind: 'ctx',  text: ' export async function loader({ params }) {' },
      { kind: 'del',  text: '-  return getUser(params.id);' },
      { kind: 'add',  text: '+  return fetchUser(params.id);' },
      { kind: 'ctx',  text: ' }' },
    ],
  },
  {
    id: 'n-004',
    agent: 'shop-agent',
    sender: 'Grocery',
    src: 'Instacart via Shopper',
    type: 'numeric',
    urgency: 2,
    blocking: false,
    sentAt: '-00:08:30',
    question: 'How many cartons of oat milk this week?',
    detail: 'You averaged 2.3 last month. Current price: $4.20.',
    min: 0, max: 8, step: 1, defaultValue: 2,
    unit: 'cartons',
  },
  {
    id: 'n-005',
    agent: 'cal-agent',
    sender: 'Calendar',
    src: 'Google Calendar via Chronos',
    type: 'schedule',
    urgency: 3,
    blocking: false,
    sentAt: '-00:11:55',
    question: 'When should I book the dentist follow-up?',
    detail: 'Dr. Wen has 5 openings in the next two weeks. Avoiding your blocked focus mornings.',
    slots: [
      { day: 'Thu May 22', time: '2:30 PM' },
      { day: 'Fri May 23', time: '11:00 AM' },
      { day: 'Mon May 26', time: '9:15 AM',  disabled: true, reason: 'focus block' },
      { day: 'Tue May 27', time: '3:45 PM' },
      { day: 'Wed May 28', time: '10:30 AM' },
      { day: 'Fri May 30', time: '1:00 PM' },
    ],
  },
  {
    id: 'n-006',
    agent: 'doc-agent',
    sender: 'Drafts',
    src: 'Gmail via Postmaster',
    type: 'freetext',
    urgency: 2,
    blocking: false,
    sentAt: '-00:14:20',
    question: 'One-line reply to the Q3 OKR thread?',
    detail: 'Sara asked if you can present the roadmap on the 28th. Tone: warm, brief.',
    placeholder: 'e.g. "Yes — 15 min slot works, I\'ll send slides Friday."',
    suggestions: [
      'Yes — works on my end, I\'ll bring slides.',
      'Can we do the 29th instead? Doctor on the 28th.',
      'Going to delegate this one to Priya.',
    ],
  },
  {
    id: 'n-007',
    agent: 'pay-agent',
    sender: 'Ledger',
    src: 'QuickBooks via Ledger',
    type: 'file',
    urgency: 3,
    blocking: false,
    sentAt: '-00:17:02',
    question: 'Drop the Q1 mileage log here.',
    detail: 'Dana needs the CSV before she finalizes the schedule C. PDF or CSV works.',
    accept: '.csv,.pdf,.xlsx',
  },
  {
    id: 'n-008',
    agent: 'guard',
    sender: 'Sentinel',
    src: 'GitHub via Sentinel',
    type: 'ack',
    urgency: 4,
    blocking: false,
    sentAt: '-00:21:48',
    question: 'New device signed into your Github.',
    detail: 'MacBook Pro · San Francisco, CA · 192.0.2.41. Looks like you — same IP as last night.',
    ackLabel: 'Got it',
  },
  {
    id: 'n-009',
    agent: 'travel',
    sender: 'Travel',
    src: 'Kayak via Tripwise',
    type: 'picklist',
    urgency: 2,
    blocking: false,
    sentAt: '-00:26:14',
    question: 'Pick a hotel for the Austin trip (Jun 12–14).',
    detail: 'Filtered to walkable to the venue, under $280/night, rating ≥ 4.4.',
    items: [
      { id: 'h1', title: 'The Driskill',           meta: '$262 · 0.3mi · ★ 4.7' },
      { id: 'h2', title: 'Hotel Saint Cecilia',    meta: '$278 · 0.8mi · ★ 4.9' },
      { id: 'h3', title: 'Hotel Magdalena',        meta: '$241 · 1.1mi · ★ 4.6' },
      { id: 'h4', title: 'Carpenter Hotel',        meta: '$219 · 1.4mi · ★ 4.5' },
      { id: 'h5', title: 'South Congress Hotel',   meta: '$255 · 1.0mi · ★ 4.6' },
      { id: 'h6', title: 'Hotel Ella',             meta: '$232 · 1.6mi · ★ 4.4' },
      { id: 'h7', title: 'The LINE Austin',        meta: '$268 · 0.5mi · ★ 4.5' },
      { id: 'h8', title: 'Austin Proper',          meta: '$279 · 0.4mi · ★ 4.7' },
    ],
  },
];

// Recently answered — feeds the history pane and the replay scrubber.
const HISTORY = [
  { id: 'h-001', agent: 'fam',        question: 'Pick up Mia from soccer at 5?',          answer: 'Yes',            type: 'yesno',       answeredAt: '4 min ago' },
  { id: 'h-002', agent: 'code-agent', question: 'Bump react-router 6 → 7?',                answer: 'No, defer',      type: 'yesno',       answeredAt: '11 min ago' },
  { id: 'h-003', agent: 'inbox-agent',question: 'Reply tone for VC intro?',                answer: 'Warm + brief',   type: 'multichoice', answeredAt: '18 min ago' },
  { id: 'h-004', agent: 'pay-agent',  question: 'Approve $42 reimbursement to Priya?',     answer: 'Yes',            type: 'yesno',       answeredAt: '32 min ago' },
  { id: 'h-005', agent: 'cal-agent',  question: 'Reschedule 1:1 with Marcus?',             answer: 'Fri 3pm',        type: 'schedule',    answeredAt: '1 hr ago' },
  { id: 'h-006', agent: 'doc-agent',  question: 'Use draft v2 of the press release?',      answer: 'Yes, with edits',answer2: '(2 lines changed)', type: 'diff', answeredAt: '1 hr ago' },
];

// Active rules — what the user has told the AI intermediate layer.
const SEED_RULES = [
  { id: 'r1', text: 'Surface texts from kids immediately',              active: true,  scope: 'fam',        urgencyMin: 3 },
  { id: 'r2', text: 'Code agent: only diffs over 10 files',             active: true,  scope: 'code-agent', urgencyMin: 2 },
  { id: 'r3', text: 'Mute Slack until 3pm',                             active: true,  scope: 'inbox-agent',urgencyMin: 0, mute: true },
  { id: 'r4', text: 'Anything from Dana is urgent',                     active: true,  scope: 'tax-agent',  urgencyMin: 4 },
];

Object.assign(window, {
  agentColor, agentColorDim, AGENTS, SEED_NOTIFICATIONS, HISTORY, SEED_RULES,
});
