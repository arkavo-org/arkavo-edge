"use strict";

function addA2AMessage(event) {
    addTelemetry({
        eventType: 'a2a-message',
        agentId: event.fromAgent || event.from_agent,
        details: { to: event.toAgent || event.to_agent, method: event.method },
        timestamp: event.timestamp || new Date().toISOString()
    });
}

function addTelemetry(event) {
    var eventType = event.eventType || event.event_type || '';

    // Store metrics snapshots in time-series buffer
    if (eventType === 'metrics_snapshot' && event.details) {
        var snap = typeof event.details === 'string' ? JSON.parse(event.details) : event.details;
        snap._receivedAt = event.timestamp || new Date().toISOString();
        AppState.lastMetricsSnapshot = snap;
        AppState.metricsHistory.push(snap);
        if (AppState.metricsHistory.length > MAX_METRICS_HISTORY) {
            AppState.metricsHistory.shift();
        }
    }

    AppState.telemetry.unshift(event);
    if (AppState.telemetry.length > MAX_TELEMETRY) {
        AppState.telemetry.pop();
    }
    AppState.eventCount++;
    renderTelemetry();
    updateStats();
}

function renderTelemetry() {
    var container = document.getElementById('telemetry-container');
    if (!container) return;

    var snap = AppState.lastMetricsSnapshot;
    var html = '';

    // Section A: System Overview stat cards
    html += '<div class="section-title">System Overview</div>';
    html += '<div class="stats-grid">';
    html += renderMetricCard('Msg/sec', snap ? snap.messages_per_second || snap.messagesPerSecond || 0 : 0, 'msgs');
    html += renderMetricCard('Memory', snap ? (snap.memory_usage_mb || snap.memoryUsageMb || 0).toFixed(1) : '—', 'MB');
    html += renderMetricCard('CPU', snap ? (snap.cpu_usage_percent || snap.cpuUsagePercent || 0).toFixed(1) : '—', '%');
    html += renderMetricCard('Error Rate', snap ? ((snap.error_rate || snap.errorRate || 0) * 100).toFixed(1) : '0', '%');
    html += '</div>';

    // Section B: Subsystem Timing Breakdown
    html += '<div class="section-title">Subsystem Timing</div>';
    html += renderSubsystemTiming(snap);

    // Section C: Throughput Sparklines
    html += '<div class="section-title">Throughput Trends</div>';
    html += renderThroughputSparklines();

    // Section D: Recent Events (last 20)
    html += '<div class="section-title">Recent Events</div>';
    html += renderRecentEvents();

    container.innerHTML = html;
}

function renderMetricCard(label, value, unit) {
    return '<div class="stat-card">' +
        '<div class="stat-card-label">' + escapeHtml(label) + '</div>' +
        '<div class="stat-card-value">' + escapeHtml(String(value)) +
        '<span class="stat-card-sub"> ' + escapeHtml(unit) + '</span></div>' +
        '</div>';
}

function renderSubsystemTiming(snap) {
    var timing = snap && (snap.subsystem_timing || snap.subsystemTiming);
    if (!timing) {
        return '<div class="empty-state"><p>No timing data yet</p></div>';
    }

    var router = timing.router_decision_avg_ms || timing.routerDecisionAvgMs || 0;
    var conductor = timing.conductor_orchestration_avg_ms || timing.conductorOrchestrationAvgMs || 0;
    var mcp = timing.mcp_tool_avg_ms || timing.mcpToolAvgMs || 0;
    var mcpP95 = timing.mcp_tool_p95_ms || timing.mcpToolP95Ms || 0;
    var inference = timing.inference_avg_ms || timing.inferenceAvgMs || 0;

    var total = router + conductor + mcp + inference;
    if (total === 0) {
        return '<div class="empty-state"><p>No timing data yet</p></div>';
    }

    // Stacked bar
    var html = '<div class="subsystem-bar">';
    html += renderBarSegment('Router', router, total, 'router');
    html += renderBarSegment('Conductor', conductor, total, 'conductor');
    html += renderBarSegment('MCP Tools', mcp, total, 'mcp');
    html += renderBarSegment('Inference', inference, total, 'inference');
    html += '</div>';

    // Legend
    html += '<div class="subsystem-legend">';
    html += '<span class="subsystem-legend-item router">Router</span>';
    html += '<span class="subsystem-legend-item conductor">Conductor</span>';
    html += '<span class="subsystem-legend-item mcp">MCP Tools</span>';
    html += '<span class="subsystem-legend-item inference">Inference</span>';
    html += '</div>';

    // Table with values
    html += '<table class="subsystem-table"><thead><tr>' +
        '<th>Subsystem</th><th>Avg (ms)</th><th>P95 (ms)</th><th>Share</th>' +
        '</tr></thead><tbody>';
    html += renderTimingRow('Router', router, '—', total);
    html += renderTimingRow('Conductor', conductor, '—', total);
    html += renderTimingRow('MCP Tools', mcp, mcpP95 > 0 ? mcpP95.toFixed(1) : '—', total);
    html += renderTimingRow('Inference', inference, '—', total);
    html += '</tbody></table>';

    return html;
}

function renderBarSegment(label, value, total, cls) {
    var pct = total > 0 ? (value / total * 100) : 0;
    if (pct < 1) return '';
    return '<div class="subsystem-bar-segment ' + cls + '" style="width:' + pct.toFixed(1) + '%"' +
        ' title="' + escapeHtml(label) + ': ' + value.toFixed(1) + 'ms">' +
        (pct >= 10 ? pct.toFixed(0) + '%' : '') +
        '</div>';
}

function renderTimingRow(name, avg, p95, total) {
    var pct = total > 0 ? (avg / total * 100).toFixed(0) + '%' : '—';
    return '<tr><td>' + escapeHtml(name) + '</td>' +
        '<td class="mono">' + avg.toFixed(1) + '</td>' +
        '<td class="mono">' + escapeHtml(String(p95)) + '</td>' +
        '<td class="mono">' + pct + '</td></tr>';
}

function renderThroughputSparklines() {
    var history = AppState.metricsHistory;
    if (history.length < 3) {
        return '<div class="empty-state"><p>Collecting data... (' + history.length + '/3 samples)</p></div>';
    }

    var msgRates = [];
    var deltaRates = [];
    for (var i = 0; i < history.length; i++) {
        msgRates.push(history[i].messages_per_second || history[i].messagesPerSecond || 0);
        deltaRates.push(history[i].deltas_per_second || history[i].deltasPerSecond || 0);
    }

    var html = '<div class="sparkline-row">';
    html += '<div class="sparkline-container">';
    html += '<div class="sparkline-label">Messages/sec</div>';
    html += renderTelemetrySparkline(msgRates, 'var(--accent)');
    html += '<div class="sparkline-value">' + msgRates[msgRates.length - 1].toFixed(1) + '</div>';
    html += '</div>';
    html += '<div class="sparkline-container">';
    html += '<div class="sparkline-label">Deltas/sec</div>';
    html += renderTelemetrySparkline(deltaRates, 'var(--success)');
    html += '<div class="sparkline-value">' + deltaRates[deltaRates.length - 1].toFixed(1) + '</div>';
    html += '</div>';
    html += '</div>';
    return html;
}

function renderTelemetrySparkline(values, color) {
    var w = 200, h = 32, pad = 2;
    var min = Math.min.apply(null, values);
    var max = Math.max.apply(null, values);
    var range = max - min || 1;

    var points = [];
    for (var i = 0; i < values.length; i++) {
        var x = pad + (i / (values.length - 1)) * (w - 2 * pad);
        var y = (h - pad) - ((values[i] - min) / range) * (h - 2 * pad);
        points.push(x.toFixed(1) + ',' + y.toFixed(1));
    }

    return '<svg class="telemetry-sparkline" viewBox="0 0 ' + w + ' ' + h + '">' +
        '<polyline points="' + points.join(' ') + '" fill="none" stroke="' + color + '" stroke-width="1.5" />' +
        '</svg>';
}

function renderRecentEvents() {
    var events = AppState.telemetry.slice(0, 20);
    if (events.length === 0) {
        return '<div class="empty-state"><p>No events yet</p></div>';
    }

    return '<div class="recent-events">' + events.map(function(evt) {
        var eventType = evt.eventType || evt.event_type || '';
        var typeClass = eventType.replace(/_/g, '-').replace(/[^a-zA-Z0-9-]/g, '');
        return '<div class="telemetry-item ' + typeClass + '">' +
            '<div class="telemetry-time">' + escapeHtml(formatTime(evt.timestamp)) + '</div>' +
            '<div class="telemetry-type">' + (escapeHtml(eventType) || 'event') + '</div>' +
            '<div class="telemetry-details">' +
            escapeHtml(evt.agentId || evt.agent_id || '') +
            (evt.details ? ' - ' + escapeHtml(JSON.stringify(evt.details).slice(0, 60)) : '') +
            '</div></div>';
    }).join('') + '</div>';
}
