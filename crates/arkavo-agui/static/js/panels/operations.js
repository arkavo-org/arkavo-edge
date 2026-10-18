"use strict";

function receivedAge(key) {
    var received = AppState.observations[key];
    if (!received) return 'Not reported';
    return '<span data-observation="' + key + '">' + observationAge(new Date(received).toISOString()) + '</span>';
}

function updateObservationAges() {
    document.querySelectorAll('[data-observation]').forEach(function(el) {
        var received = AppState.observations[el.dataset.observation];
        var age = received ? observationAge(new Date(received).toISOString()) : 'Unknown';
        el.textContent = (snapshotState(el.dataset.observation) === 'stale' ? 'Stale · ' : '') + age;
    });
    document.querySelectorAll('[data-time]').forEach(function(el) { el.textContent = observationAge(el.dataset.time); });
    updateComposerAvailability();
    document.querySelectorAll('[data-review]').forEach(function(button) {
        button.disabled = AppState.connection !== 'connected' || snapshotState('policy') !== 'current' || !!AppState.reviewPending;
    });
}

function taskRows(tasks) {
    return '<div class="ops-table-wrap"><table class="ops-table"><thead><tr><th>Task</th><th>State</th><th class="optional-column">Agent</th><th>Created</th></tr></thead><tbody>' + tasks.map(function(task) {
        var timestamp = task.created_at || task.createdAt;
        var agent = getTaskAgent(task, task.id);
        var reason = taskBlocker(task);
        return '<tr data-task-row="' + escapeHtml(task.id) + '"><td>' + routeLink('tasks', { task: task.id }, task.summary || task.description || 'Task', 'task-row-button') + '<span class="row-secondary">' + escapeHtml(shortTaskId(task.id)) + '</span></td>' +
            '<td class="task-row-state">' + uiState(task.status) + (reason ? '<span class="row-secondary">' + escapeHtml(reason) + '</span>' : '') + '</td>' +
            '<td class="optional-column muted">' + escapeHtml(agent || 'Not assigned') + '</td>' +
            '<td class="age" title="' + escapeHtml(timestamp || 'Unknown') + '" data-time="' + escapeHtml(timestamp || '') + '">' + observationAge(timestamp) + '</td></tr>';
    }).join('') + '</tbody></table></div>';
}

// What decides whether a list is drawn again: never outputs or event logs,
// which are large and change nothing in a row.
function taskListFingerprint(tasks) {
    return JSON.stringify(tasks.map(function(t) {
        return [t.id, t.status, t.summary, t.description, t.target_agent || t.targetAgent, t.created_at || t.createdAt, (t.assets || []).length];
    }));
}

// Tasks an agent reported are keyed agent:task; the short form is the task's
// own id, not the first letters of the agent's name.
function shortTaskId(id) {
    return String(id).split(':').pop().slice(0, 8);
}

function sortedTasks() {
    return Object.values(AppState.tasks).sort(function(a, b) {
        return (Date.parse(b.created_at || b.createdAt) || 0) - (Date.parse(a.created_at || a.createdAt) || 0);
    });
}

function pendingProposals() {
    var items = [];
    ((AppState.arpStatus && AppState.arpStatus.agents) || []).forEach(function(agent) {
        (agent.proposals || []).forEach(function(proposal) {
            if (proposal.state === 'proposed') items.push({ agent: agent, proposal: proposal });
        });
    });
    return items;
}

function opsMetric(label, icon, value, detail) {
    return '<div class="ops-metric"><div class="ops-metric-label">' + uiIcon(icon) + label + '</div><strong>' + escapeHtml(value) + '</strong><small>' + detail + '</small></div>';
}

function renderOverview() {
    var container = document.getElementById('overview-container');
    var tasks = sortedTasks();
    var agents = Object.values(AppState.agents);
    var connected = agents.filter(function(a) { return a.status === 'connected'; }).length;
    var running = tasks.filter(function(t) { return t.status === 'working' || t.status === 'running'; }).length;
    var attention = tasks.filter(function(t) { return ['failed', 'blocked', 'input-required', 'auth-required'].indexOf(t.status) !== -1; });
    var pending = pendingProposals();
    var html = '<div class="ops-metrics">';
    html += opsMetric('Running tasks', 'circle-play', AppState.observations.tasks ? running : 'Unknown', 'Task snapshot · ' + receivedAge('tasks'));
    html += opsMetric('Needs review', 'clipboard-pen-line', AppState.observations.policy ? pending.length : 'Unknown', 'Policy snapshot · ' + receivedAge('policy'));
    html += opsMetric('Agent links', 'network', AppState.observations.swarm ? connected + ' / ' + agents.length : 'Unknown', 'Swarm snapshot · ' + receivedAge('swarm'));
    html += opsMetric('Task exceptions', 'octagon-pause', AppState.observations.tasks ? attention.length : 'Unknown', 'Blocked or failed · ' + receivedAge('tasks'));
    html += '</div><div class="ops-grid"><div><section class="ops-section"><div class="ops-section-heading"><h2>Needs attention</h2><button data-navigate="decisions">Review decisions</button></div>';
    var notices = Object.values(AppState.notices || {});
    html += overviewNotices();
    if (pending.length) html += '<div class="observation-row"><span>' + uiState('proposed') + '</span><button class="row-action" data-navigate="decisions">' + pending.length + ' policy change' + (pending.length === 1 ? '' : 's') + '</button></div>';
    if (attention.length) html += taskRows(attention.slice(0, 5));
    if (!attention.length && !pending.length && !notices.length) html += '<div class="ops-empty"><strong>' + (AppState.observations.tasks && AppState.observations.policy ? 'No exceptions reported' : 'Awaiting observations') + '</strong>Task and policy snapshots determine this queue.</div>';
    // Operators' own tasks lead; the cycles agents run by themselves are
    // counted, not listed, so they do not bury what people asked for.
    var ours = tasks.filter(function(t) { return t.source !== 'agent'; });
    var agentCycles = tasks.length - ours.length;
    html += '</section><section class="ops-section"><div class="ops-section-heading"><h2>Recent tasks ' + uiHelp('task') + '</h2><button data-navigate="tasks">All tasks</button></div>';
    html += ours.length ? taskRows(ours.slice(0, 8)) : '<div class="ops-empty"><strong>No tasks created here yet</strong>Create a task when an agent is available.</div>';
    if (agentCycles) html += '<div class="observation-row"><span>Agent activity<small>Work agents started themselves</small></span><button class="row-action" data-navigate="tasks">' + agentCycles + ' cycle' + (agentCycles === 1 ? '' : 's') + '</button></div>';
    html += '</section>' + latestImages(tasks) + '</div><aside><section class="ops-section"><div class="ops-section-heading"><h2>Observations ' + uiHelp('freshness') + '</h2></div>';
    html += '<div class="observation-row"><span>Gateway link</span><span>' + uiState(AppState.connection) + '</span></div>';
    [['swarm', 'Swarm'], ['tasks', 'Tasks'], ['policy', 'Runtime policy']].forEach(function(item) {
        html += '<div class="observation-row"><span>' + item[1] + '<small>Gateway snapshot</small></span><span class="observation-value">' + receivedAge(item[0]) + '</span></div>';
    });
    html += '<div class="observation-row"><span>Operator grant</span><span class="muted">Not reported</span></div>';
    html += '<div class="observation-row"><span>Containment receipt</span><span class="muted">Not reported</span></div></section>';
    html += '<section class="ops-section"><div class="ops-section-heading"><h2>Evidence ' + uiHelp('session') + '</h2><button data-navigate="telemetry">Inspect</button></div><div class="observation-row"><span>Session events</span><span>' + AppState.telemetry.length + ' retained</span></div></section></aside></div>';
    // Ignore snapshot receipt times when deciding whether to rebuild interactive rows.
    var fingerprint = JSON.stringify([taskListFingerprint(tasks), agents, pending, notices.map(function(n) { return [n.id, n.revision]; }), AppState.noticeActionsPending, AppState.connection, AppState.telemetry.length, Object.keys(AppState.observations)]);
    if (container.dataset.fingerprint !== fingerprint) {
        container.innerHTML = html;
        container.dataset.fingerprint = fingerprint;
    }
    updateObservationAges();
}

function renderDecisions() {
    var container = document.getElementById('decisions-container');
    var items = pendingProposals();
    var route = AppState.route || {};
    if (route.agent || route.proposal) {
        items = [];
        ((AppState.arpStatus && AppState.arpStatus.agents) || []).forEach(function(agent) {
            if (route.agent && agent.agentId !== route.agent) return;
            (agent.proposals || []).forEach(function(proposal) {
                if (!route.proposal || proposal.id === route.proposal) items.push({ agent: agent, proposal: proposal });
            });
        });
    }
    var fingerprint = JSON.stringify([items, route, AppState.connection, AppState.reviewPending, AppState.reviewResult]);
    if (container.dataset.fingerprint === fingerprint) return;
    var open = Array.from(container.querySelectorAll('details[open]')).map(function(el) { return el.dataset.proposal; });
    var html = route.agent || route.proposal ? '<div class="inspection-toolbar">' + routeLink('decisions', {}, 'All decisions') + '<code>' + escapeHtml(route.proposal || route.agent) + '</code></div>' : '';
    html += '<section class="ops-section"><div class="ops-section-heading"><h2>' + (route.agent || route.proposal ? 'Policy changes' : 'Needs review') + '</h2><span class="muted">' + (AppState.arpStatus ? items.length : 'Unknown') + ' · ' + receivedAge('policy') + '</span></div>';
    if (AppState.reviewResult) html += '<p class="review-result" role="status">' + escapeHtml(AppState.reviewResult) + '</p>';
    if (!items.length) html += '<div class="ops-empty"><strong>' + (route.proposal ? 'Decision not in this snapshot' : AppState.arpStatus ? 'No pending policy changes' : 'Policy not reported') + '</strong>' + routeLink('arp', { agent: route.agent }, 'Inspect runtime policy') + '</div>';
    items.forEach(function(item) {
        var p = item.proposal;
        var evidence = (p.evidence || []).filter(function(e) { return e.traceId; }).map(function(e) { return routeLink('telemetry', { trace: e.traceId }, e.traceId); }).join(' ');
        html += '<details class="review-detail" data-proposal="' + escapeHtml(p.id) + '"' + (route.proposal || open.indexOf(p.id) !== -1 ? ' open' : '') + '><summary>' + uiState(p.state) + '<strong>' + escapeHtml(p.effectSummary || p.effectKind || 'Policy change') + '</strong><span class="muted">' + escapeHtml(item.agent.agentId) + '</span></summary>';
        html += '<dl><dt>Scope</dt><dd>' + routeLink('agents', { agent: item.agent.agentId }, item.agent.agentId) + ' · ' + escapeHtml(p.blastRadius || 'Not reported') + '</dd><dt>Change</dt><dd>' + escapeHtml(p.effectSummary || 'Not reported') + '</dd><dt>Reason</dt><dd>' + escapeHtml(p.rationale || 'Not reported') + '</dd><dt>Evidence</dt><dd>' + (evidence || 'Not reported') + '</dd><dt>Authority</dt><dd>Local gateway review · operator grant not verified</dd></dl>';
        html += '<div class="review-actions">';
        (p.state === 'proposed' ? ['approve', 'reject'] : []).forEach(function(action) {
            html += '<button type="button" data-review="' + action + '" data-agent="' + escapeHtml(item.agent.agentId) + '" data-proposal="' + escapeHtml(p.id) + '"' + (AppState.connection !== 'connected' || snapshotState('policy') !== 'current' || (AppState.reviewPending && !AppState.reviewPending.unknown) ? ' disabled' : '') + '>' + uiIcon(action === 'approve' ? 'check' : 'x') + (action === 'approve' ? ' Approve change' : ' Reject change') + '</button>';
        });
        html += routeLink('decisions', { agent: item.agent.agentId, proposal: p.id }, 'Direct link') + routeLink('arp', { agent: item.agent.agentId }, 'Runtime policy');
        html += '</div></details>';
    });
    container.innerHTML = html + '</section>';
    container.dataset.fingerprint = fingerprint;
    container.querySelectorAll('[data-review]').forEach(function(button) {
        button.addEventListener('click', function() {
            submitPolicyReview(button.dataset.agent, button.dataset.proposal, button.dataset.review);
        });
    });
}

function submitPolicyReview(agentId, proposalId, action) {
    if (AppState.reviewPending && !AppState.reviewPending.unknown) return false;
    if (snapshotState('policy') !== 'current') {
        AppState.reviewResult = 'Not sent · current policy snapshot required';
        renderDecisions();
        return false;
    }
    var sent = wsSend({ type: 'requestArpProposalAction', agentId: agentId, proposalId: proposalId, action: action });
    if (!sent) {
        AppState.reviewResult = 'Not sent · gateway disconnected';
    } else {
        AppState.reviewPending = { agentId: agentId, proposalId: proposalId };
        AppState.reviewResult = 'Sending · awaiting gateway receipt';
        setTimeout(function() {
            if (AppState.reviewPending && AppState.reviewPending.proposalId === proposalId) {
                // Unlock the buttons, but keep the request: a late receipt
                // still reports what the gateway did.
                AppState.reviewPending = Object.assign({}, AppState.reviewPending, { unknown: true });
                AppState.reviewResult = 'Outcome unknown · receipt not received. Inspect runtime policy before another action.';
                renderDecisions();
            }
        }, 15000);
    }
    renderDecisions();
    return sent;
}

function handlePolicyReviewResult(event) {
    var pending = AppState.reviewPending;
    if (!pending || pending.proposalId !== event.proposalId || pending.agentId !== event.agentId) return;
    AppState.reviewPending = null;
    AppState.reviewResult = event.error ? 'Rejected · ' + event.error : 'Accepted by gateway · inspect runtime policy for application state';
    renderDecisions();
    AppState.arpLastFingerprint = null;
    wsSend({ type: 'requestArpStatus' });
}
