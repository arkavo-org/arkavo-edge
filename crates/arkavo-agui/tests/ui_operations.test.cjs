const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync, readdirSync } = require('node:fs');
const { join } = require('node:path');
const vm = require('node:vm');

const root = join(__dirname, '../static/js');

function environment(...files) {
    const elements = new Map();
    const listeners = {};
    const documentListeners = {};
    const timers = [];
    const sent = [];
    const historyEntries = [];
    const location = { protocol: 'http:', host: 'localhost:7700', hash: '' };
    function element(id) {
        if (!elements.has(id)) elements.set(id, {
            id, value: '', textContent: '', innerHTML: '', dataset: {}, hidden: false,
            disabled: false, readOnly: false, open: false,
            style: {}, offsetWidth: 280, offsetHeight: 60,
            getBoundingClientRect: () => ({ left: 10, bottom: 40 }),
            matches() { return false; }, closest() { return null; },
            focus() { this.focused = true; }, showModal() { this.open = true; }, close() { this.open = false; },
            addEventListener(type, callback) { this['on' + type] = callback; },
            setAttribute(name, value) { this[name] = value; }, removeAttribute(name) { delete this[name]; },
            getAttribute(name) { return this[name] || null; },
            appendChild(child) { this.firstChild = child; child.parentElement = this; },
            querySelectorAll() { return []; }, querySelector() { return null; }, contains() { return false; },
            classList: { add() {}, remove() {}, toggle() {} }
        });
        return elements.get(id);
    }
    const context = vm.createContext({
        document: {
            getElementById: element, querySelectorAll: () => [], querySelector: () => null,
            addEventListener: (type, callback) => { documentListeners[type] = callback; },
            createElement: () => element('created-' + elements.size),
            body: { appendChild(child) { this.firstChild = child; child.parentElement = this; } }
        },
        window: { addEventListener: (type, callback) => { listeners[type] = callback; } },
        setTimeout: callback => { timers.push(callback); return timers.length; },
        clearTimeout() {}, setInterval() {}, clearInterval() {}, innerWidth: 1024, innerHeight: 800,
        location, URLSearchParams,
        history: {
            pushState(state, title, hash) { historyEntries.push(hash); location.hash = hash; },
            replaceState(state, title, hash) { location.hash = hash; }
        },
        evidenceFilter: '', evidenceDisplayed: null,
        crypto: { randomUUID: () => 'request-123' }, CSS: { escape: value => value },
        WebSocket: class { static OPEN = 1; constructor() { this.readyState = 1; } send(data) { sent.push(JSON.parse(data)); } },
        showToast(text) { context.toasts.push(text); }, toasts: [], scheduleViewRender() {},
        renderTaskDetail() {}, renderTasks() {}, renderAgents() {},
        addTelemetry() {}, updateStats() {}, switchView() {},
        activateView(view) { context.AppState.activeView = view; },
        renderOverview() {}, renderDecisions() {}, getTaskAgent: task => task.targetAgent,
        shortAgentName: id => id, getAgentChat: () => ({ messages: [], isOpen: false }),
        bindAgentChatListeners() {}, updateComposerAvailability() {}, markSubmissionUnknown() {},
        acceptTaskSubmission() {}, agentChats: {}
    });
    for (const file of ['state.js', 'icons.js', 'lexicon.js', 'inspection.js', 'routes.js', 'task-assets.js', 'notices.js', ...files]) {
        vm.runInContext(readFileSync(join(root, file), 'utf8'), context, { filename: join(root, file) });
    }
    return { context, element, listeners, documentListeners, timers, sent, historyEntries };
}

function ready(env) {
    env.context.AppState.connection = 'connected';
    env.context.AppState.observations.swarm = Date.now();
    env.context.AppState.observations.policy = Date.now();
    env.context.AppState.agents.worker = { id: 'worker', status: 'connected' };
    env.context.ws = new env.context.WebSocket();
    env.element('task-description').value = 'Inspect the local build';
}

test('all shipped scripts parse without a bundler', () => {
    for (const directory of [root, join(root, 'panels')]) {
        for (const file of readdirSync(directory).filter(file => file.endsWith('.js'))) {
            new vm.Script(readFileSync(join(directory, file), 'utf8'), { filename: file });
        }
    }
});

test('agent workspace uses Swarm and distinguishes kits from executions', () => {
    const { context: c } = environment();
    assert.equal(c.UI_TERMS.agents[0], 'Swarm');
    assert.equal(c.UI_TERMS.kit[0], 'SwarmKit');
    assert.equal(c.UI_TERMS.flight[0], 'SwarmFlight');
    assert.match(c.UI_TERMS.flight[2], /execution instance of a SwarmKit/);
    const html = readFileSync(join(root, '..', 'index.html'), 'utf8');
    assert.match(html, /data-nav-term="agents">Swarm</);
    assert.doesNotMatch(html, /\bFleet\b/);
    for (const term of Object.values(c.UI_TERMS)) assert.ok(c.UI_ICONS[term[1]], term[0]);
});

// Regression: the page's CSP (script-src 'self') refuses inline handlers, so
// a rendered onchange= never ran and logged a violation instead.
test('no shipped script renders an inline event handler', () => {
    for (const directory of [root, join(root, 'panels')]) {
        for (const file of readdirSync(directory).filter(file => file.endsWith('.js'))) {
            assert.doesNotMatch(readFileSync(join(directory, file), 'utf8'), /\son[a-z]+="/, file);
        }
    }
    assert.doesNotMatch(readFileSync(join(root, '..', 'index.html'), 'utf8'), /\son[a-z]+="/);
});

function notice(id, severity, extra) {
    return Object.assign({
        id, key: 'k-' + id, agentId: 'artist', severity, title: 'Image generation failed', text: 'The GPU ran out of memory.',
        focus: { view: 'agents', agent: 'artist' }, count: 1, revision: 1, raisedAt: '2026-10-18T00:00:00Z',
        actions: [
            { id: 'inspect', label: 'Inspect agent', kind: 'focus', focus: { view: 'agents', agent: 'artist' } },
            { id: 'acknowledge', label: 'Acknowledge', kind: 'acknowledge' }
        ]
    }, extra || {});
}

// AGUI-027: a critical notice takes the view where it points; a warning
// waits in the banner for the operator to open it.
test('a critical notice opens where it points; a warning waits', () => {
    const e = environment('websocket.js');
    e.context.AppState.activeView = 'tasks';
    e.context.handleOperatorNotice({ notice: notice('w1', 'warning') });
    assert.equal(e.historyEntries.length, 0);
    assert.match(e.element('notice-banner').innerHTML, /Image generation failed/);
    e.context.handleOperatorNotice({ notice: notice('c1', 'critical', { title: 'Data stopped at the egress gate' }) });
    assert.deepEqual(e.historyEntries, ['#/agents?agent=artist']);
    assert.match(e.element('notice-banner').innerHTML, /Data stopped at the egress gate/, 'the banner says why the view moved');
    assert.match(e.element('notices-toggle').innerHTML, /notice-count alert">2</);
});

// AGUI-027: nothing moves the view while the operator types or has a dialog
// open; the notice waits and says so.
test('a notice that arrives while the operator types waits', () => {
    const e = environment('websocket.js');
    e.context.AppState.activeView = 'tasks';
    e.context.document.activeElement = { tagName: 'TEXTAREA' };
    e.context.handleOperatorNotice({ notice: notice('c1', 'critical') });
    assert.equal(e.historyEntries.length, 0);
    assert.match(e.context.toasts[0], /waiting until you finish/);
    assert.match(e.element('notice-banner').innerHTML, /Image generation failed/);
});

// AGUI-027: what an agent writes is shown as text, never as markup.
test('notice text is escaped', () => {
    const e = environment('websocket.js');
    e.context.AppState.activeView = 'tasks';
    e.context.handleOperatorNotice({ notice: notice('w1', 'warning', { title: '<img src=x onerror=alert(1)>', details: [{ label: '<b>', value: '"x"' }] }) });
    const html = e.element('notice-banner').innerHTML;
    assert.doesNotMatch(html, /<img src=x/);
    assert.match(html, /&lt;img src=x onerror=alert\(1\)&gt;/);
    assert.match(html, /&lt;b&gt;/);
});

// AGUI-029: acknowledging goes to the gateway, which answers the agent;
// opening where a notice points happens here and is recorded there too.
test('notice actions go to the gateway; focus opens here', () => {
    const e = environment('websocket.js');
    e.context.AppState.activeView = 'tasks';
    ready(e);
    e.context.handleOperatorNotice({ notice: notice('w1', 'warning') });
    e.context.chooseNoticeAction({ dataset: { notice: 'w1', noticeAction: 'acknowledge' } });
    assert.deepEqual(e.sent.at(-1), { type: 'noticeAction', agentId: 'artist', noticeId: 'w1', actionId: 'acknowledge' });
    assert.match(e.element('notices-list').innerHTML, /Sending/);
    e.context.handleNoticeActionResult({ noticeId: 'w1', actionId: 'acknowledge' });
    e.context.handleNoticeResolved({ agentId: 'artist', noticeId: 'w1' });
    assert.doesNotMatch(e.element('notice-banner').innerHTML, /Image generation failed/);

    e.context.handleOperatorNotice({ notice: notice('w2', 'warning') });
    e.context.chooseNoticeAction({ dataset: { notice: 'w2', noticeAction: 'inspect' } });
    assert.deepEqual(e.historyEntries.at(-1), '#/agents?agent=artist');
    assert.equal(e.sent.at(-1).actionId, 'inspect');
});

test('untrusted text is escaped in both content and quoted attributes', () => {
    const { context: c } = environment();
    assert.equal(c.escapeHtml('<img src="x" onerror=\'bad\'>&'), '&lt;img src=&quot;x&quot; onerror=&#39;bad&#39;&gt;&amp;');
    assert.equal(c.escapeHtml(null), '');
    assert.equal(c.escapeHtml(0), '0');
});

test('states retain unsupported backend meanings and never infer a trusted badge', () => {
    const { context: c } = environment();
    assert.match(c.uiState('working'), /Running/);
    assert.match(c.uiState('submitted'), /Submitted/);
    assert.match(c.uiState('arbitrary <state>'), /arbitrary &lt;state&gt;/);
    assert.match(c.uiState(null), /Unknown/);
    assert.doesNotMatch(c.uiState('connected'), /Trusted|shield-check/);
    assert.equal(c.uiIcon('__proto__'), '');
    assert.equal(c.uiIcon('not-an-icon'), '');
    for (const term of Object.values(c.UI_TERMS)) assert.ok(c.UI_ICONS[term[1]], term[1]);
    for (const state of Object.values(c.UI_STATES)) if (state[1]) assert.ok(c.UI_ICONS[state[1]], state[1]);
});

test('observation age handles missing, future and expired observations', () => {
    const { context: c } = environment();
    const now = Date.parse('2026-10-17T12:00:00Z');
    assert.equal(c.observationAge(null, now), 'Unknown');
    assert.equal(c.observationAge('2026-10-18', now), 'Unknown');
    for (const [elapsed, label] of [[0, 'now'], [1500, '1s'], [120000, '2m'], [3600000, '1h'], [86400000, '1d']]) {
        assert.equal(c.observationAge(new Date(now - elapsed).toISOString(), now), label);
    }
    assert.equal(c.snapshotState('swarm', now), 'unknown');
    c.AppState.connection = 'connected';
    c.AppState.observations.swarm = now - 15000;
    assert.equal(c.snapshotState('swarm', now), 'current');
    assert.equal(c.snapshotState('swarm', now + 1), 'stale');
    c.AppState.connection = 'disconnected';
    assert.equal(c.snapshotState('swarm', now), 'stale');
});

test('closing and reopening the composer preserves the draft', () => {
    const e = environment('websocket.js', 'task-composer.js');
    e.element('task-description').value = 'Unsubmitted work';
    e.context.showAddTaskModal();
    assert.equal(e.element('add-task-modal').open, true);
    e.context.hideAddTaskModal();
    e.context.showAddTaskModal();
    assert.equal(e.element('task-description').value, 'Unsubmitted work');
});

test('offline, empty swarm and stale swarm cannot silently discard a task', () => {
    const e = environment('websocket.js', 'task-composer.js');
    ready(e);
    e.context.AppState.connection = 'disconnected';
    e.context.submitTask();
    assert.equal(e.sent.length, 0);
    assert.match(e.element('task-submission-status').textContent, /Not sent/);
    e.context.AppState.connection = 'connected';
    e.context.AppState.agents = {};
    e.context.submitTask();
    assert.equal(e.sent.length, 0);
    assert.match(e.element('task-submission-status').textContent, /no connected agents/);
    e.context.AppState.agents.worker = { status: 'connected' };
    e.context.AppState.observations.swarm = Date.now() - 16000;
    e.context.submitTask();
    assert.equal(e.sent.length, 0);
    assert.equal(e.element('task-description').value, 'Inspect the local build');
});

test('submission clears the draft only for its matching gateway receipt', () => {
    const e = environment('websocket.js', 'task-composer.js');
    ready(e);
    e.context.submitTask();
    assert.equal(e.sent.length, 1);
    assert.equal(e.sent[0].requestId, 'request-123');
    assert.equal(e.element('task-description').readOnly, true);
    e.context.acceptTaskSubmission({ requestId: 'unrelated' });
    assert.equal(e.element('task-description').value, 'Inspect the local build');
    e.context.submitTask();
    assert.equal(e.sent.length, 1);
    e.context.acceptTaskSubmission({ requestId: 'request-123' });
    assert.equal(e.element('task-description').value, '');
    assert.equal(e.element('task-description').readOnly, false);
});

// Regression (AGUI-023): an unknown outcome locked the composer until a
// reload. Reconnecting still sends nothing by itself; sending again is the
// operator's choice and reuses the request id, so the gateway answers it with
// the task it may already have opened.
test('lost acceptance becomes outcome unknown; sending again reuses the request id', () => {
    const e = environment('websocket.js', 'task-composer.js');
    ready(e);
    e.context.submitTask();
    e.context.setConnectionStatus('disconnected', 'Disconnected');
    assert.match(e.element('task-submission-status').textContent, /Outcome unknown/);
    e.context.setConnectionStatus('connected', 'Connected');
    assert.equal(e.sent.length, 1, 'reconnecting sends nothing');
    assert.equal(e.element('task-submit-btn').disabled, false, 'the composer is not locked');
    assert.equal(e.element('task-submit-btn').textContent, 'Send again');
    e.context.submitTask();
    assert.equal(e.sent.length, 2);
    assert.equal(e.sent[1].requestId, e.sent[0].requestId);
    assert.equal(e.element('task-description').value, 'Inspect the local build');
    e.context.acceptTaskSubmission({ requestId: 'request-123' });
    assert.equal(e.context.taskSubmission, null);
});

// Regression (AGUI-023): browsers provide crypto.randomUUID only on HTTPS or
// localhost, so creating a task over a LAN address threw before sending.
test('a task is created where crypto.randomUUID is absent', () => {
    const e = environment('websocket.js', 'task-composer.js');
    ready(e);
    e.context.crypto = { getRandomValues: bytes => bytes.fill(7) };
    e.context.submitTask();
    assert.equal(e.sent.length, 1);
    assert.match(e.sent[0].requestId, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});

test('receipt timeout and page unload retain unsent work', () => {
    const e = environment('websocket.js', 'task-composer.js');
    ready(e);
    let prevented = false;
    e.listeners.beforeunload({ preventDefault() { prevented = true; } });
    assert.equal(prevented, true);
    e.context.submitTask();
    e.timers[0]();
    assert.match(e.element('task-submission-status').textContent, /Outcome unknown/);
});

test('transport handles malformed frames and send failure without clearing drafts', () => {
    const e = environment('websocket.js', 'task-composer.js');
    let routed = 0;
    e.context.routeEvent = () => routed++;
    e.context.connect();
    for (const data of ['{', 'null', '123', '{"type":12}']) e.context.ws.onmessage({ data });
    e.context.ws.onmessage({ data: '{"type":"taskList","tasks":[]}' });
    assert.equal(routed, 1);
    e.context.ws.send = () => { throw new Error('closed'); };
    assert.equal(e.context.wsSend({ type: 'submitTask' }), false);
    e.context.ws = null;
    assert.equal(e.context.wsSend({ type: 'submitTask' }), false);
});

test('swarm loss retains last observations and chat history', () => {
    const e = environment('panels/tasks.js', 'panels/agents.js');
    e.context.renderAgents = () => {};
    e.context.handleMeshStatus({ agents: [{ id: 'worker', purpose: 'Review code' }] });
    e.context.agentChats.worker = { messages: ['retained'] };
    e.context.removeAgent({ agentId: 'worker' });
    assert.equal(e.context.AppState.agents.worker.status, 'disconnected');
    assert.equal(e.context.AppState.agents.worker.purpose, 'Review code');
    assert.equal(e.context.agentChats.worker.messages[0], 'retained');
    e.context.handleMeshStatus({ agents: [{ id: 'worker', status: 'connected' }] });
    assert.equal(e.context.AppState.agents.worker.status, 'connected');
    e.context.handleMeshStatus({ agents: [] });
    assert.equal(e.context.AppState.agents.worker.status, 'disconnected');
});

test('snapshot replacement preserves task evidence and excludes unsafe keys', () => {
    const e = environment('panels/tasks.js');
    e.context.renderTasks = () => {};
    e.context.AppState.tasks['task-1'] = { _events: [{ eventType: 'task-working' }], result: 'Retained output', metrics: { tokensGenerated: 42 } };
    e.context.handleTaskList({ tasks: [{ id: 'task-1', status: 'working' }, { id: '__proto__' }, { id: 1 }] });
    assert.equal(e.context.AppState.tasks['task-1']._events.length, 1);
    assert.equal(e.context.AppState.tasks['task-1'].result, 'Retained output');
    assert.equal(e.context.AppState.tasks['task-1'].metrics.tokensGenerated, 42);
    assert.equal(Object.keys(e.context.AppState.tasks).length, 1);
    assert.equal(e.context.safeTaskKey('constructor'), null);
});

test('compact task rows preserve full labels and escape record identifiers', () => {
    const { context: c } = environment('panels/operations.js');
    const html = c.taskRows([{ id: '" onclick="bad', description: '<script>bad</script>', status: 'input-required' }]);
    assert.match(html, /&lt;script&gt;/);
    assert.match(html, /&quot; onclick=&quot;bad/);
    assert.match(html, /Blocked/);
    assert.match(html, /Input required/);
    assert.doesNotMatch(html, /onclick="bad/);
});

test('policy review waits for its receipt and does not imply enforcement', () => {
    const e = environment('websocket.js', 'panels/operations.js');
    ready(e);
    e.context.renderDecisions = () => {};
    e.context.submitPolicyReview('worker', 'proposal-1', 'approve');
    e.context.submitPolicyReview('worker', 'proposal-1', 'approve');
    assert.equal(e.sent.length, 1);
    e.context.handlePolicyReviewResult({ agentId: 'other', proposalId: 'proposal-1' });
    assert.ok(e.context.AppState.reviewPending);
    e.context.handlePolicyReviewResult({ agentId: 'worker', proposalId: 'proposal-1' });
    assert.equal(e.context.AppState.reviewPending, null);
    assert.match(e.context.AppState.reviewResult, /Accepted by gateway/);
    assert.doesNotMatch(e.context.AppState.reviewResult, /Enforced/);
});

test('offline chat send preserves input and does not fabricate a sent message', () => {
    const e = environment('websocket.js', 'chat.js');
    e.element('chat-input-worker').value = 'Retain this message';
    e.context.sendAgentChatMessage('worker');
    assert.equal(e.element('chat-input-worker').value, 'Retain this message');
    assert.equal(e.context.getAgentChat('worker').messages.length, 0);
});

test('overview distinguishes unreported data from measured zeros and exposes attention', () => {
    const e = environment('panels/operations.js');
    e.context.renderOverview();
    assert.match(e.element('overview-container').innerHTML, /Unknown/);
    assert.match(e.element('overview-container').innerHTML, /Not reported/);
    e.context.AppState.observations.tasks = Date.now();
    e.context.AppState.observations.swarm = Date.now();
    e.context.AppState.observations.policy = Date.now();
    e.context.AppState.connection = 'connected';
    e.context.renderOverview();
    assert.match(e.element('overview-container').innerHTML, /No exceptions reported/);
    e.context.AppState.tasks.one = { id: 'one', status: 'working', description: 'First task', createdAt: '2026-10-17' };
    e.context.AppState.tasks.two = { id: 'two', status: 'failed', description: 'Failed task', createdAt: '2026-10-18' };
    e.context.AppState.agents.worker = { status: 'connected' };
    e.context.AppState.arpStatus = { agents: [{ agentId: 'worker', proposals: [{ id: 'p', state: 'proposed' }, { id: 'old', state: 'rejected' }] }] };
    e.context.renderOverview();
    const html = e.element('overview-container').innerHTML;
    assert.match(html, /Failed task/);
    assert.match(html, /1 policy change/);
    assert.match(html, /1 \/ 1/);
    e.context.renderOverview();
    assert.equal(e.element('overview-container').innerHTML, html);
    assert.equal(e.context.sortedTasks()[0].id, 'two');
});

test('decisions show exact scope, reason and evidence with explicit local authority', () => {
    const e = environment('panels/operations.js');
    e.context.renderDecisions();
    assert.match(e.element('decisions-container').innerHTML, /Policy not reported/);
    e.context.AppState.arpStatus = { agents: [{ agentId: 'worker', proposals: [{ id: 'p', state: 'proposed', effectSummary: 'Restrict tool access', blastRadius: 'one agent', rationale: 'Policy violation', evidence: [{ traceId: 'trace-1' }] }] }] };
    e.context.renderDecisions();
    let html = e.element('decisions-container').innerHTML;
    for (const text of ['Restrict tool access', 'one agent', 'Policy violation', 'trace-1', 'operator grant not verified', 'disabled']) assert.ok(html.includes(text));
    e.context.AppState.connection = 'connected';
    e.context.AppState.observations.policy = Date.now();
    e.context.renderDecisions();
    assert.doesNotMatch(e.element('decisions-container').innerHTML, / disabled/);
});

test('stale review is rejected locally; timeout and gateway error remain inspectable', () => {
    const e = environment('websocket.js', 'panels/operations.js');
    ready(e);
    e.context.AppState.observations.policy = Date.now() - 16000;
    e.context.submitPolicyReview('worker', 'p', 'approve');
    assert.equal(e.sent.length, 0);
    assert.match(e.context.AppState.reviewResult, /current policy snapshot required/);
    e.context.AppState.observations.policy = Date.now();
    e.context.submitPolicyReview('worker', 'p', 'approve');
    e.timers[0]();
    assert.match(e.context.AppState.reviewResult, /Outcome unknown/);
    e.context.handlePolicyReviewResult({ agentId: 'worker', proposalId: 'p', error: 'No longer proposed' });
    assert.match(e.context.AppState.reviewResult, /Rejected · No longer proposed/);
});

test('evidence filters and pauses a session snapshot while live events continue', () => {
    const e = environment('panels/evidence.js');
    e.context.renderEvidence();
    assert.match(e.element('evidence-rows').innerHTML, /No events received/);
    e.context.AppState.telemetry = [{ eventType: 'policy-denied', agentId: 'worker', timestamp: '2026-10-17T12:00:00Z', details: '<private>' }];
    e.context.renderEvidence();
    assert.match(e.element('evidence-rows').innerHTML, /&lt;private&gt;/);
    e.element('evidence-pause').onclick({ target: e.element('evidence-pause') });
    e.context.AppState.telemetry.unshift({ eventType: 'task-submitted' });
    e.context.renderEvidence();
    assert.equal(e.element('evidence-count').textContent, '1 / 1 events');
    e.element('evidence-filter').oninput({ target: { value: 'MISSING' } });
    assert.match(e.element('evidence-rows').innerHTML, /No matching events/);
    e.element('evidence-filter').oninput({ target: { value: 'policy' } });
    assert.equal(e.element('evidence-count').textContent, '1 / 1 events');
    e.element('evidence-pause').onclick({ target: e.element('evidence-pause') });
    assert.equal(e.element('evidence-count').textContent, '1 / 2 events');
});

test('empty objective, failed send and malformed socket closure preserve work', () => {
    const e = environment('websocket.js', 'task-composer.js');
    ready(e);
    e.element('task-description').value = ' ';
    e.context.submitTask();
    assert.match(e.element('task-submission-status').textContent, /Enter an objective/);
    e.element('task-description').value = 'Retain';
    e.context.ws.send = () => { throw new Error('connection closed'); };
    e.context.submitTask();
    assert.equal(e.element('task-description').value, 'Retain');
    assert.match(e.element('task-submission-status').textContent, /Not sent/);
    e.context.connect();
    e.context.ws.onopen();
    assert.equal(e.sent.length, 3, 'fleet snapshot, task list and notices on open');
    e.context.ws.onerror();
    e.context.ws.onclose();
    assert.equal(e.context.AppState.connection, 'reconnecting');
    assert.ok(e.timers.length);
});

test('information help works by hover, keyboard focus, pin and Escape', () => {
    const e = environment('help.js');
    const button = e.element('help-task');
    button.dataset.help = 'task';
    button.closest = selector => selector === 'dialog' ? null : button;
    button.matches = () => true;
    const bubble = e.context.document.body.firstChild;
    assert.equal(bubble.hidden, true);
    e.documentListeners.pointerover({ target: button });
    assert.equal(bubble.hidden, false);
    assert.match(bubble.textContent, /One unit of work/);
    assert.equal(button['aria-describedby'], 'information-help');
    e.documentListeners.click({ target: button });
    e.documentListeners.pointerout({ target: button });
    e.timers.at(-1)();
    assert.equal(bubble.hidden, false);
    e.documentListeners.keydown({ key: 'Escape', preventDefault() {} });
    assert.equal(bubble.hidden, true);
    assert.equal(button['aria-describedby'], undefined);
    e.documentListeners.focusin({ target: button });
    assert.equal(bubble.hidden, false);
    e.documentListeners.focusout();
    e.timers.at(-1)();
    assert.equal(bubble.hidden, true);
    e.documentListeners.click({ target: button });
    e.documentListeners.click({ target: button });
    assert.equal(bubble.hidden, true);
    e.listeners.resize();
});

test('information help stays hoverable and clamps to the viewport', () => {
    const e = environment('help.js');
    const button = e.element('help-scope');
    button.dataset.help = 'scope';
    button.closest = selector => selector === 'dialog' ? null : button;
    button.getBoundingClientRect = () => ({ left: 1000, bottom: 795 });
    const bubble = e.context.document.body.firstChild;
    bubble.contains = target => target === bubble;
    e.documentListeners.pointerover({ target: button });
    assert.equal(bubble.style.left, '736px');
    assert.equal(bubble.style.top, '732px');
    e.documentListeners.pointerover({ target: bubble });
    bubble.matches = () => true;
    e.documentListeners.pointerout({ target: button });
    e.timers.at(-1)();
    assert.equal(bubble.hidden, false);
    e.documentListeners.click({ target: bubble });
    assert.equal(bubble.hidden, false);
    e.documentListeners.click({ target: e.element('outside') });
    assert.equal(bubble.hidden, true);
});

test('navigation keeps all runtime panels reachable with visible workspace names', () => {
    const e = environment('navigation.js');
    let tabsBelowTitle;
    for (const view of ['budget', 'arp', 'overview']) {
        e.element('view-' + view).querySelector = selector => {
            assert.equal(selector, '.panel-header');
            return { after(tabs) {
                assert.equal(tabs, e.element('workspace-subnav'));
                tabsBelowTitle = view;
            } };
        };
    }
    for (const view of ['health', 'budget', 'router', 'learning', 'context', 'diagnostics']) assert.equal(e.context.workspaceFor(view), 'health');
    assert.equal(e.context.workspaceFor('arp'), 'security');
    assert.equal(e.context.workspaceFor('tasks'), 'tasks');
    e.context.updateNavigation('budget');
    assert.equal(tabsBelowTitle, 'budget');
    assert.match(e.element('workspace-subnav').innerHTML, /aria-current="page">Resources/);
    assert.equal(e.context.document.title, 'Runtime · Arkavo Operations');
    e.context.updateNavigation('arp');
    assert.equal(tabsBelowTitle, 'arp');
    assert.match(e.element('workspace-subnav').innerHTML, /aria-current="page">Runtime policy/);
    e.context.updateNavigation('overview');
    assert.equal(tabsBelowTitle, 'overview');
    assert.equal(e.element('workspace-subnav').innerHTML, '');
    e.context.updateObservationAges = () => {};
    e.context.initializeOperations();
    assert.match(e.element('lexicon-content').innerHTML, /Boundary/);
    e.element('lexicon-toggle').onclick();
    assert.equal(e.element('lexicon-dialog').open, true);
    e.element('lexicon-close').onclick();
    assert.equal(e.element('lexicon-dialog').open, false);
});

test('task lifecycle updates retain evidence, render blocked reasons and bound history', () => {
    const e = environment('panels/tasks.js', 'panels/operations.js');
    e.context.handleTaskSubmitted({ taskId: 'task-1', description: 'Inspect build', status: 'submitted', timestamp: '2026-10-17' });
    assert.equal(e.context.AppState.tasks['task-1'].description, 'Inspect build');
    assert.match(e.element('tasks-container').innerHTML, /Inspect build/);
    e.context.handleTaskStatusChanged({ taskId: 'task-1', status: 'input-required', timestamp: '2026-10-17', progress: 0.1 });
    assert.match(e.element('tasks-container').innerHTML, /Input required/);
    e.context.handleTaskStatusChanged({ taskId: 'task-1', status: 'completed', timestamp: '2026-10-18', result: 'Done', metrics: { tokensGenerated: 10 } });
    assert.equal(e.context.AppState.tasks['task-1'].completed_at, '2026-10-18');
    assert.equal(e.context.AppState.tasks['task-1'].result, 'Done');
    for (let i = 0; i < 60; i++) e.context.addTaskEvent('task-1', { eventType: 'update' });
    assert.equal(e.context.AppState.tasks['task-1']._events.length, 50);
    e.context.navigateRoute('tasks', { task: 'task-1' });
    e.context.handleTaskStatusChanged({ taskId: 'task-1', status: 'completed' });
    e.context.renderTasks();
    e.context.navigateRoute('tasks');
    assert.equal(e.context.AppState.selectedTaskId, null);
    e.context.handleTaskList({ tasks: [] });
    assert.match(e.element('tasks-container').innerHTML, /No tasks reported/);
});

test('task updates target the task workspace when overview has the same task', () => {
    const e = environment('panels/tasks.js');
    const cell = { innerHTML: '' };
    e.context.document.querySelector = () => { throw new Error('must not select an inactive overview row'); };
    e.element('tasks-container').querySelector = () => ({ querySelector: () => cell });
    e.context.AppState.tasks.one = { status: 'failed', result: '<unsafe>' };
    assert.equal(e.context.updateTaskCardInPlace('one'), true);
    assert.match(cell.innerHTML, /Failed/);
    assert.match(cell.innerHTML, /&lt;unsafe&gt;/);
    e.context.AppState.tasks.one.status = 'auth-required';
    e.context.updateTaskCardInPlace('one');
    assert.match(cell.innerHTML, /Authorization required/);
});

test('a malformed acknowledgement cannot discard a matching draft', () => {
    const e = environment('websocket.js', 'task-composer.js', 'panels/tasks.js');
    ready(e);
    e.context.submitTask();
    e.context.handleTaskSubmitted({ requestId: 'request-123', taskId: '__proto__' });
    assert.equal(e.element('task-description').value, 'Inspect the local build');
    assert.ok(e.context.taskSubmission);
});

test('modal information help stays inside the active accessibility subtree', () => {
    const e = environment('help.js');
    const dialog = e.element('dialog');
    const button = e.element('modal-help');
    button.dataset.help = 'task';
    button.closest = selector => selector === 'dialog' ? dialog : button;
    const bubble = e.context.document.body.firstChild;
    let shown = 0;
    bubble.showPopover = () => shown++;
    bubble.hidePopover = () => {};
    e.documentListeners.pointerover({ target: button });
    assert.equal(bubble.parentElement, dialog);
    assert.equal(shown, 1);
    assert.equal(button['aria-describedby'], bubble.id);
});

test('ending flight tracking reports registry removal without claiming cancellation', () => {
    const e = environment('websocket.js', 'panels/arp.js');
    e.context.AppState.arpStatus = { agents: [] };
    e.context.flightRequests['flight-1'] = 'Awaiting receipt';
    e.context.handleFlightStopped({ flightId: 'flight-1' });
    assert.equal(e.context.flightRequests['flight-1'], undefined);
    assert.match(e.element('arp-container').innerHTML, /Tracking ended · remote execution unchanged/);
    assert.doesNotMatch(e.element('arp-container').innerHTML, /Cancelled|Contained/);
    e.context.handleFlightStopped({ flightId: 'bad', error: 'Invalid flight identifier' });
    assert.match(e.element('arp-container').innerHTML, /End tracking rejected/);
});

test('record routes round-trip opaque identifiers without accepting arbitrary views or fields', () => {
    const { context: c } = environment();
    const id = 'worker:/team a?b&c#"<x>';
    const href = c.routeHref('decisions', { agent: id, proposal: 'p/1', action: 'approve' });
    const route = c.parseRoute(href);
    assert.equal(route.agent, id);
    assert.equal(route.proposal, 'p/1');
    assert.equal(route.action, undefined);
    assert.equal(c.parseRoute('#/constructor'), null);
    assert.equal(c.parseRoute('#/tasks?task=%00bad').task, undefined);
    assert.equal(c.parseRoute('#/tasks?task=' + 'a'.repeat(513)).task, undefined);
    assert.equal(c.parseRoute('').view, 'overview');
    assert.equal(c.routeHref('not-a-view'), '#/overview');
    const link = c.routeLink('tasks', { task: id }, '<task>', 'row');
    assert.match(link, /&lt;task&gt;/);
    assert.doesNotMatch(link, /<x>/);
});

test('browser navigation restores record scope, list position and focus without dispatching commands', () => {
    const e = environment();
    const c = e.context;
    const workspace = e.element('workspace');
    const row = e.element('task-link');
    row.href = '#/tasks?task=one';
    const title = e.element('task-title');
    e.element('view-tasks').querySelector = () => title;
    c.initializeRoutes();
    assert.equal(c.AppState.activeView, 'overview');
    c.switchView('tasks');
    workspace.scrollTop = 420;
    c.document.activeElement = row;
    c.navigateRoute('tasks', { task: 'one' });
    assert.equal(c.AppState.selectedTaskId, 'one');
    assert.equal(workspace.scrollTop, 0);
    assert.equal(title.focused, true);
    workspace.querySelector = () => row;
    c.location.hash = '#/tasks';
    e.listeners.hashchange();
    assert.equal(c.AppState.selectedTaskId, null);
    assert.equal(workspace.scrollTop, 420);
    assert.equal(row.focused, true);
    const count = e.historyEntries.length;
    c.navigateRoute('tasks');
    e.listeners.hashchange();
    assert.equal(e.historyEntries.length, count);
    c.location.hash = '#workspace';
    e.listeners.hashchange();
    assert.equal(c.AppState.activeView, 'tasks');
    c.location.hash = '#/missing';
    e.listeners.hashchange();
    assert.equal(c.AppState.activeView, 'overview');
    assert.equal(c.location.hash, '#/overview');
    assert.equal(e.sent.length, 0);
});

test('evidence scope and text filter restore independently when returning from an inspector', () => {
    const e = environment('panels/evidence.js');
    e.context.navigateRoute('telemetry', { task: 'one' });
    e.element('evidence-filter').oninput({ target: { value: 'denied' } });
    e.context.navigateRoute('agents', { agent: 'worker' });
    e.context.location.hash = '#/telemetry?task=one';
    e.context.initializeRoutes();
    assert.equal(e.context.evidenceFilter, 'denied');
    assert.equal(e.element('evidence-filter').value, 'denied');
    assert.equal(e.context.AppState.route.task, 'one');
});

test('task inspection survives an absent initial snapshot and a later list-detail round trip', () => {
    const e = environment('panels/tasks.js', 'panels/task-detail.js', 'panels/operations.js');
    const c = e.context;
    c.AppState.selectedTaskId = 'one';
    c.renderTaskDetail();
    assert.equal(c.AppState.selectedTaskId, 'one');
    assert.match(e.element('tasks-container').innerHTML, /Awaiting gateway snapshot/);
    c.AppState.observations.tasks = Date.now();
    c.handleTaskList({ tasks: [{ id: 'one', description: 'Inspect <build>', status: 'blocked' }] });
    const detail = e.element('tasks-container').innerHTML;
    assert.match(detail, /Inspect &lt;build&gt;/);
    assert.match(detail, /Reason not reported/);
    assert.match(detail, /Task policy binding/);
    c.AppState.selectedTaskId = null;
    c.renderTasks();
    assert.match(e.element('tasks-container').innerHTML, /ops-table/);
    c.AppState.selectedTaskId = 'one';
    c.renderTaskDetail();
    assert.equal(e.element('tasks-container').innerHTML, detail);
    c.handleTaskList({ tasks: [] });
    assert.match(e.element('tasks-container').innerHTML, /Record unavailable/);
    assert.equal(c.AppState.selectedTaskId, 'one');
});

test('task inspection separates summary from output and does not invent missing measurements', () => {
    const e = environment('panels/task-detail.js', 'panels/operations.js');
    const c = e.context;
    c.AppState.selectedTaskId = 'one';
    c.AppState.tasks.one = { id: 'one', description: 'Objective', summary: 'Summary only', status: 'completed', targetAgent: 'worker', metrics: {} };
    c.renderTaskDetail();
    let html = e.element('tasks-container').innerHTML;
    assert.doesNotMatch(html, /Summary only|0\.0 Wh|proven performer/);
    assert.match(html, /Output<\/summary>.*Not reported/);
    assert.match(html, /#\/arp\?agent=worker/);
    c.AppState.tasks.one.result = '<output>';
    c.AppState.tasks.one.metrics.energyWh = 0;
    c.AppState.arpStatus = { agents: [{ agentId: 'worker', documentValid: true }] };
    c.renderTaskDetail();
    html = e.element('tasks-container').innerHTML;
    assert.match(html, /&lt;output&gt;/);
    assert.match(html, /0\.0 Wh/);
    assert.match(html, /Document valid/);
    assert.match(html, /Task policy binding<\/dt><dd>Not reported/);
    assert.equal(c.taskMetric(NaN, 'Wh'), 'Not reported');
    assert.equal(c.taskMetric(-1, 'Wh'), 'Not reported');
    assert.equal(c.formatMetricMs(250), '250 ms');
    assert.equal(c.formatMetricMs(2500), '2.5 s');
    assert.equal(c.formatMetricMs(62500), '1 min 2 s');
    assert.equal(c.formatMetricMs(undefined), 'Not reported');
});

test('task route uses the latest matching record and keeps unsupported strategy or quality unknown', () => {
    const { context: c } = environment('panels/task-detail.js');
    c.AppState.routingHistory = [{ taskId: 'one', selectedAgent: 'old' }, { taskId: 'other', selectedAgent: 'wrong' }, { taskId: 'one', selectedAgent: 'new', wasExploration: false, outcome: '<failed>', qualityScore: 0, qualityIssues: ['<issue>'] }];
    assert.equal(c.getTaskAgent({}, 'one'), 'new');
    assert.equal(c.getTaskAgent({ targetAgent: 'assigned' }, 'one'), 'assigned');
    let html = c.renderTaskRouting('one', {});
    assert.match(html, /Exploitation/);
    assert.match(html, /0%/);
    assert.match(html, /&lt;issue&gt;/);
    assert.match(html, /&lt;failed&gt;/);
    c.AppState.routingHistory[2].wasExploration = true;
    assert.match(c.renderTaskRouting('one', {}), /Exploration/);
    assert.doesNotMatch(c.renderTaskRouting('missing', { metrics: { qualityScore: 5 } }), /500%/);
});

test('task evidence never matches agent proximity, substring IDs or identifiers buried in text', () => {
    const { context: c } = environment();
    assert.equal(c.eventMatchesScope({ agentId: 'worker', timestamp: 'now', details: { text: 'task-one' } }, { task: 'task-one' }), false);
    assert.equal(c.eventMatchesScope({ details: { taskId: 'task-one-more' } }, { task: 'task-one' }), false);
    assert.equal(c.eventMatchesScope({ details: '{"taskId":"task-one"}' }, { task: 'task-one' }), false);
    assert.equal(c.eventMatchesScope({ details: { task_id: 'task-one' } }, { task: 'task-one' }), true);
    assert.equal(c.eventMatchesScope({ taskId: 'task-one', agentId: 'different' }, { task: 'task-one', agent: 'worker' }), false);
    assert.equal(c.eventMatchesScope({ agentId: 'from', details: { to: 'worker', traceId: 'trace-one' } }, { agent: 'worker', trace: 'trace-one' }), true);
});

test('evidence reading holds rows steady, exposes pending updates and reports retention gaps', () => {
    const e = environment('panels/evidence.js');
    const c = e.context;
    const rows = e.element('evidence-rows');
    c.AppState.route = { view: 'telemetry', task: 'one' };
    c.AppState.telemetry = [{ eventType: 'task-blocked', taskId: 'one', agentId: 'worker', timestamp: '2026-10-17' }];
    c.renderEvidence();
    const original = rows.innerHTML;
    rows.querySelector = () => ({ open: true });
    c.AppState.telemetry.unshift({ eventType: 'task-completed', details: { taskId: 'one' } });
    c.renderEvidence();
    assert.equal(rows.innerHTML, original);
    assert.equal(e.element('evidence-updates').hidden, false);
    e.element('evidence-updates').onclick();
    assert.match(rows.innerHTML, /task-completed/);
    assert.equal(e.element('evidence-updates').hidden, true);
    assert.equal(rows.focused, true);
    c.AppState.evictedEvents = 7;
    c.renderEvidence();
    assert.match(e.element('evidence-retention').textContent, /7 events evicted/);
    assert.match(e.element('evidence-scope').innerHTML, /Task ·/);
    assert.match(rows.innerHTML, /#\/tasks\?task=one/);
    assert.match(rows.innerHTML, /#\/agents\?agent=worker/);
});

test('agent inspection links only assigned tasks and proposals for the exact agent', () => {
    const e = environment('panels/task-detail.js', 'panels/operations.js', 'panels/agent-detail.js');
    const c = e.context;
    c.AppState.selectedAgentId = 'worker';
    c.renderAgentDetail();
    assert.match(e.element('agent-inspector').innerHTML, /Agent not in this snapshot/);
    c.AppState.agents.worker = { id: 'worker', model: '<model>', status: 'disconnected' };
    c.AppState.tasks.one = { id: 'one', description: 'Assigned work', targetAgent: 'worker' };
    c.AppState.tasks.two = { id: 'two', description: 'Other work', targetAgent: 'other' };
    c.AppState.arpStatus = { agents: [{ agentId: 'worker', proposals: [{ id: 'p', state: 'proposed', effectSummary: 'Restrict tool' }] }] };
    c.renderAgentDetail();
    const html = e.element('agent-inspector').innerHTML;
    assert.match(html, /Assigned work/);
    assert.doesNotMatch(html, /Other work/);
    assert.match(html, /&lt;model&gt;/);
    assert.match(html, /#\/decisions\?agent=worker&amp;proposal=p/);
    assert.match(html, /Not reported/);
    c.AppState.observations.policy = Date.now();
    c.renderAgentDetail();
    assert.match(e.element('agent-inspector').innerHTML, /data-observation="policy"/);
    c.AppState.selectedAgentId = null;
    c.renderAgentDetail();
    assert.equal(e.element('agent-inspector').innerHTML, '');
});

test('decision deep links retain disposition after review and never expose another agent proposal', () => {
    const e = environment('panels/operations.js');
    const c = e.context;
    c.AppState.route = { view: 'decisions', agent: 'worker', proposal: 'p' };
    c.AppState.arpStatus = { agents: [{ agentId: 'worker', proposals: [{ id: 'p', state: 'rejected', effectSummary: 'Exact change' }] }, { agentId: 'other', proposals: [{ id: 'p', state: 'proposed', effectSummary: 'Wrong change' }] }] };
    c.renderDecisions();
    const html = e.element('decisions-container').innerHTML;
    assert.match(html, /Exact change/);
    assert.match(html, /Rejected/);
    assert.doesNotMatch(html, /Wrong change|data-review=/);
    c.AppState.route.proposal = 'missing';
    c.renderDecisions();
    assert.match(e.element('decisions-container').innerHTML, /Decision not in this snapshot/);
});

test('an unavailable agent policy never falls back to a different agent', () => {
    const e = environment('panels/arp.js');
    e.context.AppState.route = { view: 'arp', agent: 'missing' };
    e.context.AppState.arpSelectedAgent = 'missing';
    e.context.renderArp();
    assert.match(e.element('arp-container').innerHTML, /missing/);
    e.context.AppState.arpStatus = { agents: [{ agentId: 'other', documentValid: true }] };
    e.context.renderArp();
    assert.match(e.element('arp-container').innerHTML, /Policy not reported/);
    assert.doesNotMatch(e.element('arp-container').innerHTML, /other/);
});

test('inspection updates preserve disclosure state and keyboard focus', () => {
    const e = environment();
    const container = e.element('inspector');
    const detail = { dataset: { section: 'output' }, open: false };
    const replacement = e.element('replacement');
    container.querySelectorAll = () => [detail];
    container.querySelector = () => replacement;
    container.contains = () => true;
    const focused = e.element('focused-link');
    focused.href = '#/tasks';
    e.context.document.activeElement = focused;
    e.context.updateInspector(container, 'updated', 'revision-1');
    assert.equal(replacement.open, false);
    assert.equal(replacement.focused, true);
    e.context.updateInspector(container, 'must not replace', 'revision-1');
    assert.equal(container.innerHTML, 'updated');
});

test('polling an absent task leaves its navigation controls intact', () => {
    const e = environment('panels/task-detail.js');
    e.context.AppState.selectedTaskId = 'missing';
    e.context.AppState.observations.tasks = 100;
    e.context.renderTaskDetail();
    const container = e.element('tasks-container');
    const fingerprint = container.dataset.detailFingerprint;
    let replacements = 0;
    Object.defineProperty(container, 'innerHTML', { set() { replacements++; } });
    e.context.AppState.observations.tasks = 5100;
    e.context.renderTaskDetail();
    assert.equal(container.dataset.detailFingerprint, fingerprint);
    assert.equal(replacements, 0);
});

test('navigating between agent inspectors preserves each unsent chat draft', () => {
    const e = environment('panels/agents.js', 'chat.js');
    const c = e.context;
    c.AppState.agents.one = { id: 'one' };
    c.AppState.agents.two = { id: 'two' };
    const first = e.element('chat-input-one');
    first.dataset.agentId = 'one';
    first.value = 'Keep first draft';
    const second = e.element('chat-input-two');
    second.dataset.agentId = 'two';
    second.value = 'Keep second draft';
    let inputs = [first, second];
    e.element('agents-container').querySelectorAll = () => inputs;
    c.AppState.selectedAgentId = 'one';
    c.renderAgents();
    assert.equal(c.getAgentChat('one').draft, 'Keep first draft');
    assert.equal(c.getAgentChat('two').draft, 'Keep second draft');
    inputs = [first];
    c.AppState.selectedAgentId = 'two';
    c.renderAgents();
    assert.equal(c.getAgentChat('one').draft, 'Keep first draft');
    assert.equal(c.getAgentChat('two').draft, 'Keep second draft');
    assert.doesNotMatch(e.element('agents-container').innerHTML, /chat-area-one/);
    assert.match(e.element('agents-container').innerHTML, /chat-area-two/);
});

test('reconnect refreshes the active inspector without clearing its route or adding history', () => {
    const e = environment('websocket.js');
    e.context.navigateRoute('tasks', { task: 'one' });
    const historyLength = e.historyEntries.length;
    let refreshed;
    e.context.activateView = view => { refreshed = view; };
    e.context.connect();
    e.context.ws.onopen();
    assert.equal(refreshed, 'tasks');
    assert.equal(e.context.AppState.selectedTaskId, 'one');
    assert.equal(e.historyEntries.length, historyLength);
    assert.equal(e.context.location.hash, '#/tasks?task=one');
});

test('task lifecycle records keep their explicit source and escaped event text', () => {
    const { context: c } = environment('panels/task-detail.js');
    const html = c.renderTaskTelemetry('one', { _events: [{ timestamp: '2026-10-17', event_type: '<blocked>', agentId: '<worker>' }] });
    assert.match(html, /&lt;blocked&gt;/);
    assert.match(html, /&lt;worker&gt;/);
    assert.match(c.renderTaskTelemetry('one', {}), /No lifecycle events received/);
});
