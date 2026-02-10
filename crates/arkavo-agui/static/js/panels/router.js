"use strict";

function handleModelSelected(event) {
    AppState.modelDecisions.unshift({
        timestamp: new Date().toISOString(),
        agentId: event.agentId || event.agent_id,
        provider: event.provider,
        model: event.model,
        reason: event.reason,
        estimatedCost: event.estimatedCost
    });
    if (AppState.modelDecisions.length > MAX_DECISIONS) {
        AppState.modelDecisions.pop();
    }
    renderRouter();
}

function renderRouter() {
    var container = document.getElementById('router-container');
    if (!container) return;

    var db = AppState.roiDashboard;
    var html = '';

    // Model distribution bar
    if (db && db.model_distribution) {
        var md = db.model_distribution;
        html += '<div class="section-title">Model Distribution</div>';
        html += '<div class="stacked-bar">';
        if (md.local_percent > 0) {
            html += '<div class="stacked-bar-segment local" style="width:' + md.local_percent + '%">' +
                (md.local_percent > 10 ? md.local_percent.toFixed(0) + '% Local' : '') + '</div>';
        }
        if (md.cloud_percent > 0) {
            html += '<div class="stacked-bar-segment cloud" style="width:' + md.cloud_percent + '%">' +
                (md.cloud_percent > 10 ? md.cloud_percent.toFixed(0) + '% Cloud' : '') + '</div>';
        }
        html += '</div>';

        // Tasks by model table
        if (md.tasks_by_model && Object.keys(md.tasks_by_model).length > 0) {
            html += '<div class="section-title">Tasks by Model</div>';
            html += '<table class="cost-table"><thead><tr><th>Model</th><th>Tasks</th></tr></thead><tbody>';
            Object.keys(md.tasks_by_model).forEach(function(model) {
                html += '<tr><td>' + escapeHtml(model) + '</td><td>' + md.tasks_by_model[model] + '</td></tr>';
            });
            html += '</tbody></table>';
        }
    }

    // Decision log
    html += '<div class="section-title">Decision Log</div>';

    if (AppState.modelDecisions.length === 0) {
        html += '<div class="empty-state"><h3>No Decisions Yet</h3><p>Model routing decisions will appear here</p></div>';
    } else {
        html += '<div class="decision-log">';
        AppState.modelDecisions.forEach(function(d) {
            var costStr = '';
            if (d.estimatedCost) {
                var total = (d.estimatedCost.input_cost || 0) + (d.estimatedCost.output_cost || 0);
                costStr = formatCost(total);
            }
            html += '<div class="decision-item">' +
                '<div class="decision-time">' + escapeHtml(formatTime(d.timestamp)) + '</div>' +
                '<div class="decision-model">' + escapeHtml(d.model) + ' <span class="llm-provider">(' + escapeHtml(d.provider) + ')</span></div>' +
                '<div class="decision-reason">' + escapeHtml(d.reason) + '</div>' +
                (costStr ? '<div class="decision-cost">' + costStr + '</div>' : '') +
                '</div>';
        });
        html += '</div>';
    }

    container.innerHTML = html;
}
