"use strict";

// One presentation contract for navigation, state and contextual help.
var UI_TERMS = {
    overview: ['Overview', 'layout-dashboard', 'Current tasks, review requests and gateway observations.'],
    tasks: ['Missions', 'target', 'This gateway tracks standalone tasks. Durable mission records are not yet available.'],
    agents: ['Swarm', 'network', 'Agents in this gateway scope. Discovery does not establish identity or authority.'],
    kit: ['SwarmKit', 'file', 'A portable package defining coordinated agent work: roles, skills, tool grants and provisioning.'],
    flight: ['SwarmFlight', 'workflow', 'One execution instance of a SwarmKit. A standalone task is not a SwarmFlight.'],
    decisions: ['Decisions', 'git-pull-request', 'Runtime policy proposals requiring a human choice.'],
    security: ['Data & Trust', 'shield', 'Inspect protected data, policy and their reported enforcement boundaries.'],
    telemetry: ['Evidence', 'files', 'Session events supporting an observation. Events alone do not prove enforcement.'],
    health: ['Runtime', 'cpu', 'Execution resources, model routing and diagnostics.'],
    task: ['Task', 'list-todo', 'One unit of work. Submitted means recorded by this gateway, not completed by an agent.'],
    run: ['Run', 'workflow', 'One execution attempt of a task. A retry is another attempt.'],
    agent: ['Agent', 'bot', 'An identified worker that accepts work. Link state does not establish authority.'],
    grant: ['Grant', 'key-round', 'Authority for a principal to act on a resource, with an action, scope and expiry.'],
    event: ['Event', 'activity', 'A recorded observation or transition. It is not automatically proof of enforcement.'],
    receipt: ['Receipt', 'receipt-text', 'A response from the component handling an action. Acceptance and enforcement are separate claims.'],
    artifact: ['Artifact', 'file', 'A file or other addressable output. Protection and provenance are separate fields.'],
    boundary: ['Boundary', 'fence', 'A named point that enforces a specific rule, such as tool execution or key release.'],
    gateway: ['Gateway', 'plug', 'The browser connection to this gateway. Agent links and authorization are separate.'],
    freshness: ['Observation age', 'history', 'Age since this browser received the named snapshot. A snapshot can contain older source data. Live snapshots become stale after 15 seconds without a reply.'],
    policy: ['Runtime policy', 'scroll-text', 'Agent Runtime Policy (ARP): rules and proposals reported for a named agent.'],
    authority: ['Review authority', 'key-round', 'The current gateway records a local reviewer label; it does not authenticate an operator grant.'],
    tracking: ['End tracking', 'x', 'Removes this flight from the gateway registry. It does not cancel remote work or enforce containment.'],
    session: ['Session evidence', 'activity', 'The latest 200 events received in this browser. This is not a durable or verified evidence archive.'],
    scope: ['Scope', 'scan', 'This gateway and the agents it discovers. No mission or organization filter is applied.'],
    cost: ['Reported spend', 'cpu', 'Gateway-reported spend. Unknown until a budget snapshot arrives; not a complete cost of ownership.'],
    'task-policy': ['Task policy binding', 'scroll-text', 'The agent snapshot describes its currently reported policy. This gateway does not report which policy revision governed a specific task.'],
    correlation: ['Evidence scope', 'files', 'Task and trace filters match explicit identifiers in event fields. Agent activity alone does not establish a relationship to a task.'],
    energy: ['Estimated energy', 'cpu', 'The gateway may estimate energy using inference duration and an assumed 150 W GPU. This is not a power measurement.'],
    'task-metrics': ['Reported metrics', 'activity', 'Gateway metrics may include estimates, including token counts derived from output length. Setup latency is not a measured time to first token.'],
    'route-strategy': ['Route strategy', 'workflow', 'Exploration tests an agent; exploitation uses current routing scores. Neither strategy proves that the result is correct.']
};

var UI_STATES = {
    connected: ['Connected', 'plug', 'good'],
    disconnected: ['Disconnected', 'unplug', 'warn'],
    reconnecting: ['Reconnecting', 'unplug', 'warn'],
    connecting: ['Connecting', 'plug', 'neutral'],
    submitted: ['Submitted', 'inbox', 'neutral'],
    queued: ['Queued', '', 'neutral'],
    working: ['Running', 'circle-play', 'info'],
    running: ['Running', 'circle-play', 'info'],
    'input-required': ['Blocked', 'octagon-pause', 'warn'],
    'auth-required': ['Blocked', 'octagon-pause', 'warn'],
    blocked: ['Blocked', 'octagon-pause', 'warn'],
    completed: ['Completed', 'circle-check', 'good'],
    failed: ['Failed', 'circle-x', 'bad'],
    canceled: ['Cancelled', '', 'neutral'],
    cancelled: ['Cancelled', '', 'neutral'],
    proposed: ['Needs review', 'clipboard-pen-line', 'warn'],
    accepted: ['Accepted', 'inbox', 'info'],
    rejected: ['Rejected', 'ban', 'bad'],
    stale: ['Stale', 'history', 'warn'],
    unknown: ['Unknown', 'circle-help', 'muted'],
    unconfirmed: ['Unconfirmed', 'shield-question', 'warn']
};

function uiIcon(name) {
    if (!Object.prototype.hasOwnProperty.call(UI_ICONS, name)) return '';
    return '<svg class="ui-icon" aria-hidden="true" focusable="false" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' + UI_ICONS[name] + '</svg>';
}

function uiState(value) {
    var state = Object.prototype.hasOwnProperty.call(UI_STATES, value) ? UI_STATES[value] : null;
    if (!state) state = value ? [String(value), '', 'neutral'] : UI_STATES.unknown;
    return '<span class="state state-' + state[2] + '">' + uiIcon(state[1]) + escapeHtml(state[0]) + '</span>';
}

function uiHelp(key) {
    var term = Object.prototype.hasOwnProperty.call(UI_TERMS, key) ? UI_TERMS[key] : null;
    if (!term) return '';
    return '<button type="button" class="info-button" data-help="' + key + '" aria-label="About ' + escapeHtml(term[0]) + '">' + uiIcon('info') + '</button>';
}

function observationAge(timestamp, now) {
    var time = Date.parse(timestamp);
    var elapsed = (now === undefined ? Date.now() : now) - time;
    if (!Number.isFinite(time) || elapsed < 0) return 'Unknown';
    if (elapsed < 1000) return 'now';
    if (elapsed < 60000) return Math.floor(elapsed / 1000) + 's';
    if (elapsed < 3600000) return Math.floor(elapsed / 60000) + 'm';
    if (elapsed < 86400000) return Math.floor(elapsed / 3600000) + 'h';
    return Math.floor(elapsed / 86400000) + 'd';
}

function snapshotState(key, now) {
    var received = AppState.observations[key];
    if (!received) return 'unknown';
    if (AppState.connection !== 'connected' || (now || Date.now()) - received > 15000) return 'stale';
    return 'current';
}
