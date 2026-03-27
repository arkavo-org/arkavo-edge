"use strict";

function renderTaskDetail() {
    var container = document.getElementById('tasks-container');
    if (!container) return;

    var taskId = AppState.selectedTaskId;
    var task = AppState.tasks[taskId];
    if (!task) {
        deselectTask();
        return;
    }

    var html = '';
    html += '<div class="task-detail-back" id="task-detail-back-btn">&#x2190; Back to Task List</div>';
    html += renderTaskSummary(task, taskId);

    if (task.metrics) {
        html += renderTaskMetrics(task);
    }

    html += renderTaskRouting(taskId, task);
    html += renderTaskA2ATrace(taskId, task);
    html += renderTaskTelemetry(taskId, task);

    container.innerHTML = html;

    var backBtn = document.getElementById('task-detail-back-btn');
    if (backBtn) {
        backBtn.addEventListener('click', deselectTask);
    }
}

function getTaskAgent(task, taskId) {
    var agent = task.target_agent || task.targetAgent;
    if (agent) return agent;
    for (var i = 0; i < AppState.routingHistory.length; i++) {
        if (AppState.routingHistory[i].taskId === taskId) {
            return AppState.routingHistory[i].selectedAgent;
        }
    }
    return null;
}

function formatDuration(startTs, endTs) {
    if (!startTs || !endTs) return null;
    var ms = new Date(endTs) - new Date(startTs);
    if (ms < 0 || isNaN(ms)) return null;
    if (ms < 1000) return ms + 'ms';
    var secs = Math.floor(ms / 1000);
    if (secs < 60) return secs + 's';
    var mins = Math.floor(secs / 60);
    secs = secs % 60;
    return mins + 'm ' + secs + 's';
}

function renderTaskSummary(task, taskId) {
    var status = escapeHtml(task.status || '');
    var statusClass = status.replace(/[^a-zA-Z0-9-]/g, '');
    var agent = getTaskAgent(task, taskId);

    var html = '<div class="detail-section-title">Task Summary</div>';
    html += '<div class="task-detail-summary">';
    html += '<div class="task-detail-header">';
    html += '<span class="task-detail-id">#' + escapeHtml((task.id || '').slice(0, 8)) + '</span>';
    html += '<span class="task-status ' + statusClass + '">' + status + '</span>';
    html += '</div>';
    html += '<div class="stats-grid">';
    html += renderStatCard('Agent', agent ? escapeHtml(agent) : 'Pending', '');
    html += renderStatCard('Created', (task.created_at || task.createdAt) ? formatTime(task.created_at || task.createdAt) : '-', '');

    var duration = formatDuration(task.created_at || task.createdAt, task.completed_at);
    if (task.completed_at) {
        html += renderStatCard('Completed', formatTime(task.completed_at), duration ? 'in ' + duration : '');
    } else {
        var elapsed = formatDuration(task.created_at || task.createdAt, new Date().toISOString());
        html += renderStatCard('Elapsed', elapsed || '-', 'running');
    }

    var progress = typeof task.progress === 'number' ? (Math.min(100, Math.max(0, task.progress * 100)).toFixed(0) + '%') : '-';
    html += renderStatCard('Progress', progress, '');
    html += '</div>';

    if (typeof task.progress === 'number' && task.status !== 'completed' && task.status !== 'failed') {
        var pct = Math.min(100, Math.max(0, task.progress * 100));
        html += '<div class="task-progress"><div class="task-progress-bar" style="width:' + pct + '%"></div></div>';
    }

    html += '</div>';

    // Input section
    html += '<div class="detail-section-title">Input</div>';
    html += '<div class="task-detail-io">';
    html += '<pre class="task-io-text">' + escapeHtml(task.description || 'No input') + '</pre>';
    html += '</div>';

    // Output section
    html += '<div class="detail-section-title">Output</div>';
    html += '<div class="task-detail-io">';
    if (task.result) {
        html += '<pre class="task-io-text">' + escapeHtml(task.result) + '</pre>';
    } else if (task.summary) {
        html += '<pre class="task-io-text">' + escapeHtml(task.summary) + '</pre>';
    } else {
        html += '<div class="detail-empty">' + (task.status === 'completed' ? 'No output recorded' : 'Waiting for output...') + '</div>';
    }
    html += '</div>';

    return html;
}

function renderTaskResult(task) {
    var html = '<div class="detail-section-title">Result</div>';
    html += '<div class="task-detail-result">';
    html += '<pre class="task-result-text">' + escapeHtml(task.result) + '</pre>';
    html += '</div>';
    return html;
}

// Configurable energy defaults
var ENERGY_GPU_WATTS = 150;        // GPU power draw during inference (W)
var ENERGY_COST_PER_KWH = 0.12;    // Electricity cost ($/kWh)

function renderTaskMetrics(task) {
    var m = task.metrics;
    var html = '<div class="detail-section-title">Inference Metrics</div>';
    html += '<div class="task-detail-metrics">';
    html += '<div class="stats-grid">';

    // Tokens/sec
    var tokSec = m.tokensPerSec !== undefined ? m.tokensPerSec.toFixed(1) + ' tok/s' : '-';
    html += renderStatCard('Throughput', tokSec, m.tokensGenerated ? '~' + m.tokensGenerated + ' tokens' : '');

    // TTFT (A2A setup latency — proxy for actual TTFT)
    var ttft = m.ttftMs !== undefined ? formatMetricMs(m.ttftMs) : '-';
    html += renderStatCard('TTFT', ttft, 'A2A setup latency');

    // Inference duration
    var dur = m.inferenceDurationMs !== undefined ? formatMetricMs(m.inferenceDurationMs) : '-';
    html += renderStatCard('Inference Time', dur, '');

    // Energy usage
    var energyWh = m.energyWh !== undefined ? m.energyWh : 0;
    var energyDisplay = energyWh < 1 ? (energyWh * 1000).toFixed(1) + ' mWh' : energyWh.toFixed(3) + ' Wh';
    var energyCost = (energyWh / 1000) * ENERGY_COST_PER_KWH;
    var costDisplay = energyCost < 0.01 ? '<$0.01' : '$' + energyCost.toFixed(4);
    html += renderStatCard('Energy', energyDisplay, costDisplay + ' @ $' + ENERGY_COST_PER_KWH.toFixed(2) + '/kWh');

    html += '</div>';

    // GPU power config note
    html += '<div class="task-metrics-config">' + ENERGY_GPU_WATTS + 'W GPU assumed</div>';

    html += '</div>';
    return html;
}

function formatMetricMs(ms) {
    if (ms < 1000) return ms + 'ms';
    var secs = ms / 1000;
    if (secs < 60) return secs.toFixed(1) + 's';
    var mins = Math.floor(secs / 60);
    secs = secs % 60;
    return mins + 'm ' + Math.floor(secs) + 's';
}

function renderTaskRouting(taskId, task) {
    var routingEntry = null;
    for (var i = 0; i < AppState.routingHistory.length; i++) {
        if (AppState.routingHistory[i].taskId === taskId) {
            routingEntry = AppState.routingHistory[i];
            break;
        }
    }

    var html = '<div class="detail-section-title">Routing Decision</div>';

    // Resolve quality score: routing entry > task metrics > null
    var taskQuality = (task.metrics && task.metrics.qualityScore != null)
        ? task.metrics.qualityScore : null;

    if (!routingEntry) {
        if (taskQuality != null) {
            html += '<div class="task-detail-routing"><div class="stats-grid">';
            html += renderStatCard('Quality', (taskQuality * 100).toFixed(0) + '%', '');
            html += '</div></div>';
        } else {
            html += '<div class="detail-empty">No routing data recorded for this task</div>';
        }
        return html;
    }

    html += '<div class="task-detail-routing">';
    html += '<div class="stats-grid">';
    html += renderStatCard('Selected Agent', escapeHtml(routingEntry.selectedAgent), '');
    html += renderStatCard('Strategy', routingEntry.wasExploration ? 'Exploration' : 'Exploitation', routingEntry.wasExploration ? 'trying new agent' : 'proven performer');

    var outcome = routingEntry.outcome || 'pending';
    var outcomeClass = outcome === 'success' ? 'val-success' : (outcome === 'failed' ? 'val-error' : (outcome === 'degraded' ? 'val-warning' : ''));
    html += renderStatCard('Outcome', outcome, '');

    var qRaw = (routingEntry.qualityScore != null) ? routingEntry.qualityScore : taskQuality;
    var qScore = (qRaw != null) ? (qRaw * 100).toFixed(0) + '%' : 'Pending';
    html += renderStatCard('Quality', qScore, '');
    html += '</div>';

    if (routingEntry.category) {
        html += '<div class="task-detail-meta">Category: <span class="category-badge">' +
            escapeHtml(routingEntry.category) + '</span></div>';
    }

    if (routingEntry.qualityIssues && routingEntry.qualityIssues.length > 0) {
        html += '<div class="task-detail-issues-title">Quality Issues</div>';
        html += '<ul class="task-detail-issues">';
        for (var j = 0; j < routingEntry.qualityIssues.length; j++) {
            html += '<li>' + escapeHtml(routingEntry.qualityIssues[j]) + '</li>';
        }
        html += '</ul>';
    }

    html += '</div>';
    return html;
}

function renderTaskA2ATrace(taskId, task) {
    var html = '<div class="detail-section-title">A2A Message Trace</div>';

    var agent = getTaskAgent(task, taskId);
    var createdAt = task.created_at || task.createdAt;
    var completedAt = task.completed_at || new Date().toISOString();

    if (!agent) {
        html += '<div class="detail-empty">Agent not yet assigned — trace unavailable</div>';
        return html;
    }

    var a2aMessages = [];
    for (var i = 0; i < AppState.telemetry.length; i++) {
        var evt = AppState.telemetry[i];
        if (evt.eventType !== 'a2a-message') continue;

        var fromMatch = evt.agentId === agent;
        var toMatch = evt.details && evt.details.to === agent;
        if (!fromMatch && !toMatch) continue;

        if (createdAt && evt.timestamp < createdAt) continue;
        if (completedAt && evt.timestamp > completedAt) continue;

        a2aMessages.push(evt);
    }

    if (a2aMessages.length === 0) {
        html += '<div class="detail-empty">No A2A messages matched for agent <strong>' + escapeHtml(agent) + '</strong></div>';
        return html;
    }

    a2aMessages.reverse();

    html += '<div class="task-detail-a2a">';
    for (var k = 0; k < a2aMessages.length; k++) {
        var msg = a2aMessages[k];
        var direction = msg.agentId === agent ? 'outbound' : 'inbound';
        html += '<div class="a2a-trace-item ' + direction + '">';
        html += '<span class="a2a-trace-time">' + escapeHtml(formatTime(msg.timestamp)) + '</span>';
        html += '<span class="a2a-trace-dir">' + (direction === 'inbound' ? '&#x2192;' : '&#x2190;') + '</span>';
        html += '<span class="a2a-trace-from">' + escapeHtml(msg.agentId || '') + '</span>';
        html += '<span class="a2a-trace-method">' + escapeHtml((msg.details && msg.details.method) || '') + '</span>';
        html += '</div>';
    }
    html += '</div>';
    return html;
}

function renderTaskTelemetry(taskId, task) {
    var html = '<div class="detail-section-title">Task Events</div>';

    var taskEvents = (task && task._events) ? task._events : [];

    if (taskEvents.length === 0) {
        html += '<div class="detail-empty">No lifecycle events recorded</div>';
        return html;
    }

    html += '<div class="task-detail-telemetry">';
    for (var j = 0; j < taskEvents.length; j++) {
        var e = taskEvents[j];
        var eventType = e.eventType || e.event_type || '';
        var typeClass = eventType.replace(/_/g, '-').replace(/[^a-zA-Z0-9-]/g, '');
        html += '<div class="telemetry-item ' + typeClass + '">';
        html += '<div class="telemetry-time">' + escapeHtml(formatTime(e.timestamp)) + '</div>';
        html += '<div class="telemetry-type">' + escapeHtml(eventType) + '</div>';
        html += '<div class="telemetry-details">' + escapeHtml(e.agentId || '') + '</div>';
        html += '</div>';
    }
    html += '</div>';
    return html;
}
