// Utility functions for SJBIS dashboard.
// No mock data here — all data comes from the API.

// Deterministic palette per agent id. Uses a curated set of well-separated
// hues (instead of a raw hash % 360, which can map different agents to nearly
// identical hues) plus a per-agent lightness offset, so cards from different
// agents read as clearly distinct colors on the dark field.
const AGENT_HUES = [130, 175, 245, 295, 330, 25, 55, 90, 150, 210];
const AGENT_LIGHTS = [72, 78, 84];

function agentHash(id) {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h;
}
function agentHue(id) {
  return AGENT_HUES[agentHash(id) % AGENT_HUES.length];
}
function agentLight(id) {
  return AGENT_LIGHTS[(agentHash(id) >>> 8) % AGENT_LIGHTS.length];
}
function agentColor(id) {
  return `oklch(${agentLight(id)}% 0.21 ${agentHue(id)})`;
}
function agentColorDim(id) {
  return `oklch(${agentLight(id) - 20}% 0.16 ${agentHue(id)})`;
}

// These will be populated from /state API response
const AGENTS = {};
const SEED_NOTIFICATIONS = [];
const HISTORY = [];
const SEED_RULES = [];

Object.assign(window, {
  agentColor, agentColorDim, AGENTS, SEED_NOTIFICATIONS, HISTORY, SEED_RULES,
});
