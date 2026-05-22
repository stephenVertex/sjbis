// Utility functions for SJBIS dashboard.
// No mock data here — all data comes from the API.

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

// These will be populated from /state API response
const AGENTS = {};
const SEED_NOTIFICATIONS = [];
const HISTORY = [];
const SEED_RULES = [];

Object.assign(window, {
  agentColor, agentColorDim, AGENTS, SEED_NOTIFICATIONS, HISTORY, SEED_RULES,
});
