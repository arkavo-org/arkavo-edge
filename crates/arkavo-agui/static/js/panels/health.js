"use strict";

function handleStatusUpdate(event) {
    AppState.lastStatusUpdate = event;
    renderHealth();

    // Also add telemetry for heartbeat
    addTelemetry({
        eventType: 'heartbeat',
        agentId: 'system',
        details: { health: event.health && event.health.status, components: event.health && event.health.components && event.health.components.length },
        timestamp: event.timestamp || new Date().toISOString()
    });
}

function handleSystemNotification(event) {
    showToast(event.message, event.severity || 'info');
}

function renderHealth() {
    var container = document.getElementById('health-container');
    if (!container) return;

    var su = AppState.lastStatusUpdate;
    if (!su) {
        container.innerHTML = '<div class="empty-state"><h3>Waiting for Status</h3><p>Status updates arrive every 30 seconds</p></div>';
        return;
    }

    var html = '';

    // Component health cards
    if (su.health && su.health.components && su.health.components.length > 0) {
        html += '<div class="section-title">Components</div>';
        html += '<div class="health-grid">';
        su.health.components.forEach(function(c) {
            html += '<div class="health-card">' +
                '<div class="health-card-header">' +
                '<span class="health-card-name">' + escapeHtml(c.component) + '</span>' +
                '<span class="status-dot ' + escapeHtml(c.status) + '"></span>' +
                '</div>' +
                '<div class="health-card-msg">' + escapeHtml(c.message) + '</div>' +
                '</div>';
        });
        html += '</div>';
    }

    // Overall status
    if (su.health) {
        html += '<div class="section-title">Overall: ' + escapeHtml(su.health.status) + '</div>';
    }

    // LLM availability
    if (su.llms && su.llms.length > 0) {
        html += '<div class="section-title">LLM Availability</div>';
        html += '<div class="llm-list">';
        su.llms.forEach(function(llm) {
            var connClass = llm.connected ? 'llm-connected' : 'llm-disconnected';
            var connIcon = llm.connected ? '\u2713' : '\u2717';
            html += '<div class="llm-item">' +
                '<span class="' + connClass + '">' + connIcon + '</span>' +
                '<span class="llm-name">' + escapeHtml(llm.name) + '</span>' +
                '<span class="llm-provider">' + escapeHtml(llm.provider) + '</span>' +
                '<span>' + escapeHtml(llm.model) + '</span>' +
                '</div>';
        });
        html += '</div>';
    }

    // System info
    if (su.system) {
        html += '<div class="section-title">System</div>';
        html += '<div class="sys-info">';
        html += '<div class="sys-info-label">Uptime</div><div class="sys-info-value">' + escapeHtml(su.system.uptime) + '</div>';
        html += '<div class="sys-info-label">Memory</div><div class="sys-info-value">' + escapeHtml(su.system.memory_usage) + '</div>';
        html += '<div class="sys-info-label">Connections</div><div class="sys-info-value">' + su.system.active_connections + '</div>';
        html += '</div>';
    }

    // Performance (subsystem timing from metrics)
    var snap = AppState.lastMetricsSnapshot;
    var timing = snap && (snap.subsystem_timing || snap.subsystemTiming);
    if (timing) {
        html += '<div class="section-title">Performance</div>';
        html += '<div class="perf-grid">';
        var routerAvg = timing.router_decision_avg_ms || timing.routerDecisionAvgMs || 0;
        var conductorAvg = timing.conductor_orchestration_avg_ms || timing.conductorOrchestrationAvgMs || 0;
        var mcpAvg = timing.mcp_tool_avg_ms || timing.mcpToolAvgMs || 0;
        var mcpP95 = timing.mcp_tool_p95_ms || timing.mcpToolP95Ms || 0;
        var inferenceAvg = timing.inference_avg_ms || timing.inferenceAvgMs || 0;
        html += '<div class="stat-card"><div class="stat-card-label">Router Avg</div><div class="stat-card-value">' + routerAvg.toFixed(1) + '<span class="stat-card-sub"> ms</span></div></div>';
        html += '<div class="stat-card"><div class="stat-card-label">Conductor Avg</div><div class="stat-card-value">' + conductorAvg.toFixed(1) + '<span class="stat-card-sub"> ms</span></div></div>';
        html += '<div class="stat-card"><div class="stat-card-label">MCP Avg</div><div class="stat-card-value">' + mcpAvg.toFixed(1) + '<span class="stat-card-sub"> ms</span></div></div>';
        html += '<div class="stat-card"><div class="stat-card-label">MCP P95</div><div class="stat-card-value">' + mcpP95.toFixed(1) + '<span class="stat-card-sub"> ms</span></div></div>';
        html += '<div class="stat-card"><div class="stat-card-label">Inference Avg</div><div class="stat-card-value">' + inferenceAvg.toFixed(1) + '<span class="stat-card-sub"> ms</span></div></div>';
        html += '</div>';
    }

    // MCP Tools
    if (su.mcpTools) {
        html += '<div class="section-title">MCP Tools</div>';
        html += '<div class="sys-info">';
        html += '<div class="sys-info-label">Browser</div><div class="sys-info-value">' + (su.mcpTools.browser_available ? 'Available' : 'N/A') + '</div>';
        html += '<div class="sys-info-label">Tools</div><div class="sys-info-value">' + su.mcpTools.tools_count + '</div>';
        if (su.mcpTools.last_used) {
            html += '<div class="sys-info-label">Last Used</div><div class="sys-info-value">' + escapeHtml(su.mcpTools.last_used) + '</div>';
        }
        html += '</div>';
    }

    container.innerHTML = html;
}
