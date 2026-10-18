"use strict";

function renderTaskDetail() {
    var container = document.getElementById('tasks-container');
    var taskId = AppState.selectedTaskId;
    var task = AppState.tasks[taskId];
    delete container.dataset.fingerprint;
    if (!task) {
        updateInspector(container, renderMissingRecord('Task', taskId, 'tasks', 'tasks'),
            JSON.stringify([taskId, !!AppState.observations.tasks]));
        return;
    }
    requestTaskDetail(task);
    var agent = getTaskAgent(task, taskId);
    var route = latestTaskRoute(taskId);
    var events = AppState.telemetry.filter(function(event) { return eventMatchesScope(event, { task: taskId }); });
    var policy = ((AppState.arpStatus && AppState.arpStatus.agents) || []).find(function(item) { return item.agentId === agent; });
    var html = '<div class="inspection-toolbar">' + routeLink('tasks', {}, 'All tasks') +
        routeLink('telemetry', { task: taskId }, 'Task evidence') +
        (agent ? routeLink('agents', { agent: agent }, 'Inspect agent') : '') + '</div>';
    html += '<section class="ops-section"><div class="ops-section-heading"><h2>Task <code>' + escapeHtml(taskId) + '</code></h2>' + uiState(task.status) + '</div>';
    var blocker = taskBlocker(task);
    if (blocker) html += '<p class="inspection-blocker">' + escapeHtml(blocker) + '</p>';
    html += '<dl class="inspection-fields">' + inspectionField('Agent', agent ? routeLink('agents', { agent: agent }, agent) : 'Not assigned');
    html += inspectionField('Created', escapeHtml(task.created_at || task.createdAt || 'Not reported'));
    html += inspectionField('Completed', escapeHtml(task.completed_at || task.completedAt || 'Not reported'));
    html += inspectionField('Snapshot ' + uiHelp('freshness'), receivedAge('tasks')) + '</dl></section>';
    html += inspectionSection('objective', 'Objective', '<pre class="inspection-text">' + escapeHtml(task.description || 'Not reported') + '</pre>', true);
    var output = task.result ? escapeHtml(task.result) : task.hasResult ? 'Loading output' : 'Not reported';
    html += inspectionSection('output', 'Output', '<pre class="inspection-text">' + output + '</pre>' + taskGallery(task), true);
    html += renderTaskRouting(taskId, task);
    // A help control inside <summary> nests one control in another; it
    // leads the section's body instead.
    html += inspectionSection('policy', 'Agent policy',
        uiHelp('task-policy') + '<dl class="inspection-fields">' + inspectionField('Task policy binding', 'Not reported') +
        inspectionField('Agent snapshot', policy ? (policy.documentValid === true ? 'Document valid' : policy.documentValid === false ? 'Document invalid or absent' : 'Not reported') : 'Not reported') +
        inspectionField('Observed', receivedAge('policy')) + '</dl>' +
        (agent ? routeLink('arp', { agent: agent }, 'Inspect agent policy') : ''), false);
    if (task.metrics) html += renderTaskMetrics(task);
    html += inspectionSection('events', 'Task events · ' + (task._events || []).length, renderTaskTelemetry(taskId, task), false);
    html += inspectionSection('evidence', 'Session evidence · ' + events.length,
        uiHelp('correlation') + routeLink('telemetry', { task: taskId }, 'Inspect task evidence'), false);
    var shown = Object.assign({}, task, { _events: (task._events || []).length });
    updateInspector(container, html, JSON.stringify([taskId, shown, route, agent, policy, events.length, !!AppState.observations.tasks, !!AppState.observations.policy]));
}

function getTaskAgent(task, taskId) {
    var route = latestTaskRoute(taskId);
    return task.target_agent || task.targetAgent || (route && route.selectedAgent) || null;
}

function formatMetricMs(ms) {
    if (!Number.isFinite(ms) || ms < 0) return 'Not reported';
    if (ms < 1000) return ms + ' ms';
    if (ms < 60000) return (ms / 1000).toFixed(1) + ' s';
    return Math.floor(ms / 60000) + ' min ' + Math.floor(ms % 60000 / 1000) + ' s';
}

function taskMetric(value, unit) {
    return Number.isFinite(value) && value >= 0 ? escapeHtml(value.toFixed(1) + ' ' + unit) : 'Not reported';
}

function renderTaskMetrics(task) {
    var m = task.metrics;
    var html = '<dl class="inspection-fields">';
    html += inspectionField('Throughput', taskMetric(m.tokensPerSec, 'tok/s'));
    html += inspectionField('Tokens', taskMetric(m.tokensGenerated, 'tokens'));
    html += inspectionField('Setup latency', formatMetricMs(m.ttftMs));
    html += inspectionField('Inference', formatMetricMs(m.inferenceDurationMs));
    html += inspectionField('Estimated energy ' + uiHelp('energy'), taskMetric(m.energyWh, 'Wh'));
    html += '</dl>';
    return inspectionSection('metrics', 'Reported metrics', uiHelp('task-metrics') + html, false);
}

function renderTaskRouting(taskId, task) {
    var entry = latestTaskRoute(taskId);
    var html = '<dl class="inspection-fields">';
    html += inspectionField('Selected agent', escapeHtml(entry && entry.selectedAgent || getTaskAgent(task, taskId) || 'Not reported'));
    html += inspectionField('Strategy ' + uiHelp('route-strategy'), !entry || typeof entry.wasExploration !== 'boolean' ? 'Not reported' : entry.wasExploration ? 'Exploration' : 'Exploitation');
    html += inspectionField('Category', escapeHtml(entry && entry.category || 'Not reported'));
    html += inspectionField('Outcome', escapeHtml(entry && entry.outcome || 'Not reported'));
    var quality = entry && entry.qualityScore != null ? entry.qualityScore : task.metrics && task.metrics.qualityScore;
    html += inspectionField('Quality', Number.isFinite(quality) && quality >= 0 && quality <= 1 ? Math.round(quality * 100) + '%' : 'Not reported');
    html += '</dl>';
    if (entry && entry.qualityIssues && entry.qualityIssues.length) {
        html += '<ul class="inspection-issues">' + entry.qualityIssues.map(function(issue) { return '<li>' + escapeHtml(issue) + '</li>'; }).join('') + '</ul>';
    }
    return inspectionSection('route', 'Route', html, true);
}

function renderTaskTelemetry(taskId, task) {
    var events = task._events || [];
    if (!events.length) return '<p class="muted">No lifecycle events received</p>';
    return '<ol class="inspection-timeline">' + events.map(function(event) {
        return '<li><time>' + escapeHtml(formatTime(event.timestamp)) + '</time><strong>' +
            escapeHtml(event.eventType || event.event_type || 'Event') + '</strong><span>' + escapeHtml(event.agentId || 'Not reported') + '</span></li>';
    }).join('') + '</ol>';
}

// The task list leaves outputs out; an opened task asks for its own.
function requestTaskDetail(task) {
    if (!task || task.result || !task.hasResult || task._detailRequested) return;
    task._detailRequested = wsSend({ type: 'requestTask', taskId: task.id });
}
