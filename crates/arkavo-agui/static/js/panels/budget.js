"use strict";

function handleBudgetStatusUpdate(event) {
    AppState.budgetStatus = event.status;
    renderBudget();
}

function handleBudgetAlert(event) {
    showToast(event.alert.message || 'Budget alert', 'warning');
    renderBudget();
}

function handleSpendingRecorded(event) {
    if (event.record && event.record.cost) {
        AppState.totalCost += event.record.cost;
        updateCostBadge();
    }
}

function handleROIDashboardUpdate(event) {
    AppState.roiDashboard = event.dashboard;
    if (event.dashboard.session_stats) {
        AppState.totalCost = event.dashboard.session_stats.total_cost;
        updateCostBadge();
    }
    renderBudget();
}

function handleCostMetricsUpdate(event) {
    AppState.costMetrics = event.metrics;
    renderBudget();
}

function handleComputeBudgetUpdate(event) {
    if (!AppState.computeBudgets) AppState.computeBudgets = {};
    AppState.computeBudgets[event.agentId] = event.computeBudget;
    renderBudget();
}

function handleAgentSystemMetrics(event) {
    AppState.agentSystemMetrics[event.agentId] = {
        rssMb: event.rssMb,
        cpuPercent: event.cpuPercent,
        pid: event.pid,
        totalRamMb: event.totalRamMb,
        availableRamMb: event.availableRamMb
    };
    renderBudget();
}

function requestBudgetData() {
    wsSend({ type: 'getROIDashboard' });
    wsSend({ type: 'getCostMetrics', timeRange: '24h' });
    wsSend({ type: 'getBudgetStatus', agentId: null });
    wsSend({ type: 'requestMeshStatus' });
}

function updateCostBadge() {
    var badge = document.getElementById('cost-badge');
    if (badge) {
        badge.textContent = formatCost(AppState.totalCost);
    }
}

function renderBudget() {
    var container = document.getElementById('budget-container');
    if (!container) return;

    var db = AppState.roiDashboard;
    var html = '';

    if (!db) {
        html = '<div class="empty-state"><h3>Loading Budget Data...</h3><p>Requesting dashboard metrics</p></div>';
        container.innerHTML = html;
        return;
    }

    var bh = db.budget_health;
    var ss = db.session_stats;

    // Budget gauge
    var usagePct = bh ? Math.min(100, bh.usage_percent) : 0;
    var statusClass = bh ? bh.status.toLowerCase() : 'healthy';
    html += renderBudgetGauge(usagePct, statusClass);

    // Alert banner
    if (bh && bh.status !== 'Healthy') {
        html += '<div class="budget-alert ' + statusClass + '">' +
            escapeHtml(bh.status) + ': ' + usagePct.toFixed(1) + '% budget used</div>';
    }

    // Session stats grid
    html += '<div class="section-title">Session</div>';
    html += '<div class="stats-grid">';
    html += renderStatCard('Tasks', ss ? ss.total_tasks : 0, '');
    html += renderStatCard('Cost', formatCost(ss ? ss.total_cost : 0), '');
    html += renderStatCard('Savings', ss ? ss.savings_percent.toFixed(1) + '%' : '0%', '');
    html += renderStatCard('Burn Rate', formatCost(ss ? ss.burn_rate_per_hour : 0) + '/hr', '');
    html += '</div>';

    // System Memory Overview (from agentSystemMetrics)
    var sysMetrics = AppState.agentSystemMetrics;
    var sysKeys = Object.keys(sysMetrics);
    if (sysKeys.length > 0) {
        var firstMetrics = sysMetrics[sysKeys[0]];
        var totalRam = firstMetrics.totalRamMb;
        var availRam = firstMetrics.availableRamMb;
        if (totalRam) {
            var usedRam = totalRam - (availRam || 0);
            var ramPct = (usedRam / totalRam) * 100;
            html += '<div class="section-title">System Memory</div>';
            html += '<div class="stats-grid">';
            html += renderStatCard('Total RAM', formatBytes(totalRam * 1024 * 1024), '');
            html += renderStatCard('Available', formatBytes((availRam || 0) * 1024 * 1024), '');
            html += renderStatCard('Used', formatBytes(usedRam * 1024 * 1024), ramPct.toFixed(0) + '%');
            html += '</div>';
            html += '<div style="margin:0.5em 0 1em">' + renderMemoryBar(ramPct, sysKeys, sysMetrics) + '</div>';
        }
    }

    // Compute Budget Telemetry (from agents via JSON-RPC)
    var budgets = AppState.computeBudgets;
    if (budgets && Object.keys(budgets).length > 0) {
        html += '<div class="section-title">Compute Budget Telemetry</div>';
        html += '<table class="cost-table"><thead><tr>' +
            '<th>Agent</th><th>Status</th><th>Inferences</th><th>Tokens</th><th>Memory</th><th>RSS</th><th>TTL</th>' +
            '</tr></thead><tbody>';
        Object.keys(budgets).forEach(function(agentId) {
            var data = budgets[agentId];
            var cb = data.compute_budget || data;
            var status = cb.status || 'unknown';
            var statusColor = status === 'active' ? '#4ade80' :
                              status === 'exhausted' ? '#f87171' : '#94a3b8';
            var ttl = cb.ttl_remaining_secs != null ? cb.ttl_remaining_secs.toFixed(0) + 's' : '-';
            var memMax = cb.max_memory_bytes || 0;
            var memUsed = cb.used_memory_bytes || 0;
            var memLabel = memMax > 0
                ? formatBytes(memUsed) + ' / ' + formatBytes(memMax)
                : '-';
            var agentMet = sysMetrics[data.agent_id || agentId];
            var rssLabel = agentMet ? formatBytes(agentMet.rssMb * 1024 * 1024) : '-';
            html += '<tr>';
            html += '<td>' + escapeHtml(data.agent_id || agentId) + '</td>';
            html += '<td><span style="color:' + statusColor + ';font-weight:bold">' + escapeHtml(status) + '</span></td>';
            html += '<td class="mono">' + (cb.remaining_inferences || 0) + '</td>';
            html += '<td class="mono">' + (cb.remaining_tokens || 0) + '</td>';
            html += '<td class="mono">' + memLabel + '</td>';
            html += '<td class="mono">' + rssLabel + '</td>';
            html += '<td class="mono">' + ttl + '</td>';
            html += '</tr>';
        });
        html += '</tbody></table>';
    }

    // Cost breakdown by model
    var cb = db.cost_breakdown;
    if (cb && cb.by_model) {
        html += '<div class="section-title">Cost by Model</div>';
        html += '<table class="cost-table"><thead><tr><th>Model</th><th>Cost</th></tr></thead><tbody>';
        Object.keys(cb.by_model).forEach(function(model) {
            html += '<tr><td>' + escapeHtml(model) + '</td><td class="mono">' + formatCost(cb.by_model[model]) + '</td></tr>';
        });
        html += '</tbody></table>';
    }

    // Cloud vs Local split
    if (cb && cb.cloud_vs_local) {
        var cvl = cb.cloud_vs_local;
        var localPct = (cvl.local_tasks + cvl.cloud_tasks) > 0 ?
            (cvl.local_tasks / (cvl.local_tasks + cvl.cloud_tasks)) * 100 : 0;
        var cloudPct = 100 - localPct;

        html += '<div class="section-title">Cloud vs Local</div>';
        html += '<div class="split-bar">' +
            '<div class="split-bar-segment local" style="width:' + localPct + '%">' + (localPct > 10 ? localPct.toFixed(0) + '%' : '') + '</div>' +
            '<div class="split-bar-segment cloud" style="width:' + cloudPct + '%">' + (cloudPct > 10 ? cloudPct.toFixed(0) + '%' : '') + '</div>' +
            '</div>';
        html += '<div class="split-bar-legend">' +
            '<span class="local-label">Local (' + cvl.local_tasks + ' tasks)</span>' +
            '<span class="cloud-label">Cloud (' + cvl.cloud_tasks + ' tasks)</span></div>';
    }

    // Runway
    if (bh && bh.estimated_runway_hours < 1e10) {
        html += '<div class="section-title">Runway</div>';
        html += '<div class="stats-grid">';
        html += renderStatCard('Hours Left', bh.estimated_runway_hours.toFixed(1), '');
        html += renderStatCard('Tasks Left', bh.tasks_remaining < 1e8 ? bh.tasks_remaining : 'Unlimited', '');
        html += '</div>';
    }

    // Per-Agent Budget Allocations
    if (db.agent_allocations && db.agent_allocations.length > 0) {
        html += '<div class="section-title">Agent Budget Allocation</div>';
        html += '<table class="cost-table"><thead><tr><th>Agent</th><th>Allocated</th><th>Spent</th><th>Remaining</th><th>Usage</th><th>Status</th></tr></thead><tbody>';
        db.agent_allocations.forEach(function(agent) {
            var usagePct = agent.allocated_usd > 0 ? (agent.spent_usd / agent.allocated_usd) * 100 : 0;
            var statusColor = agent.status === 'active' ? '#4ade80' :
                              agent.status === 'exhausted' ? '#f87171' :
                              agent.status === 'passive' ? '#94a3b8' : '#fbbf24';
            html += '<tr>';
            html += '<td>' + escapeHtml(agent.agent_id) + '</td>';
            html += '<td class="mono">' + formatCost(agent.allocated_usd) + '</td>';
            html += '<td class="mono">' + formatCost(agent.spent_usd) + '</td>';
            html += '<td class="mono">' + formatCost(agent.remaining_usd) + '</td>';
            html += '<td>' + renderMiniGauge(usagePct) + '</td>';
            html += '<td><span style="color:' + statusColor + '">' + escapeHtml(agent.status) + '</span></td>';
            html += '</tr>';
        });
        html += '</tbody></table>';
    }

    // Recommendations
    if (db.recommendations && db.recommendations.length > 0) {
        html += '<div class="section-title">Recommendations</div>';
        html += '<ul class="recommendations-list">';
        db.recommendations.forEach(function(rec) {
            html += '<li>' + escapeHtml(rec) + '</li>';
        });
        html += '</ul>';
    }

    container.innerHTML = html;
}

function renderMiniGauge(pct) {
    pct = Math.min(100, Math.max(0, pct));
    var color = pct < 50 ? '#4ade80' : pct < 80 ? '#fbbf24' : '#f87171';
    return '<div style="display:inline-block;width:60px;height:8px;background:#334155;border-radius:4px;overflow:hidden">' +
        '<div style="width:' + pct.toFixed(0) + '%;height:100%;background:' + color + '"></div>' +
        '</div> <span class="mono" style="font-size:0.75em">' + pct.toFixed(0) + '%</span>';
}

function renderBudgetGauge(pct, statusClass) {
    var radius = 70;
    var circumference = Math.PI * radius;
    var offset = circumference - (pct / 100) * circumference;
    return '<div class="budget-gauge">' +
        '<svg viewBox="0 0 180 100">' +
        '<path class="gauge-track" d="M 10 90 A 70 70 0 0 1 170 90" />' +
        '<path class="gauge-fill ' + statusClass + '" d="M 10 90 A 70 70 0 0 1 170 90" ' +
        'stroke-dasharray="' + circumference + '" stroke-dashoffset="' + offset + '" />' +
        '<text class="gauge-text" x="90" y="78">' + pct.toFixed(0) + '%</text>' +
        '<text class="gauge-label" x="90" y="94">budget used</text>' +
        '</svg></div>';
}

function renderStatCard(label, value, sub) {
    return '<div class="stat-card">' +
        '<div class="stat-card-label">' + escapeHtml(label) + '</div>' +
        '<div class="stat-card-value">' + escapeHtml(String(value)) + '</div>' +
        (sub ? '<div class="stat-card-sub">' + escapeHtml(sub) + '</div>' : '') +
        '</div>';
}

function formatBytes(bytes) {
    if (bytes == null || bytes === 0) return '0 B';
    var gb = bytes / (1024 * 1024 * 1024);
    if (gb >= 1) return gb.toFixed(1) + ' GB';
    var mb = bytes / (1024 * 1024);
    if (mb >= 1) return mb.toFixed(0) + ' MB';
    return (bytes / 1024).toFixed(0) + ' KB';
}

function renderMemoryBar(usedPct, agentIds, metrics) {
    var barColor = usedPct < 60 ? '#4ade80' : usedPct < 85 ? '#fbbf24' : '#f87171';
    var bar = '<div style="height:18px;background:#334155;border-radius:4px;overflow:hidden;position:relative">' +
        '<div style="width:' + Math.min(100, usedPct).toFixed(1) + '%;height:100%;background:' + barColor +
        ';transition:width 0.3s"></div></div>';
    var legend = '<div style="display:flex;gap:1em;flex-wrap:wrap;margin-top:0.25em;font-size:0.75em;color:#94a3b8">';
    for (var i = 0; i < agentIds.length; i++) {
        var m = metrics[agentIds[i]];
        if (m && m.rssMb) {
            legend += '<span>' + escapeHtml(agentIds[i]) + ': ' + formatBytes(m.rssMb * 1024 * 1024) +
                (m.cpuPercent != null ? ' · ' + m.cpuPercent.toFixed(0) + '% CPU' : '') + '</span>';
        }
    }
    legend += '</div>';
    return bar + legend;
}
