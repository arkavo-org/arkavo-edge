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

function requestBudgetData() {
    wsSend({ type: 'getROIDashboard' });
    wsSend({ type: 'getCostMetrics', timeRange: '24h' });
    wsSend({ type: 'getBudgetStatus', agentId: null });
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
