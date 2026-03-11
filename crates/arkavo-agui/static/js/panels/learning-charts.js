"use strict";

// Learning Charts — quality trends, summary bar, sparklines

function successRateColor(rate) {
    if (rate < 0.5) {
        return 'var(--error)';
    } else if (rate < 0.75) {
        return 'var(--warning)';
    }
    return 'var(--success)';
}

function renderSparkline(scores) {
    if (!scores || scores.length < 2) return '<span class="trend-no-data">-</span>';

    var w = 80, h = 20, pad = 2;
    var min = Math.min.apply(null, scores);
    var max = Math.max.apply(null, scores);
    var range = max - min || 1;

    var points = [];
    for (var i = 0; i < scores.length; i++) {
        var x = pad + (i / (scores.length - 1)) * (w - 2 * pad);
        var y = (h - pad) - ((scores[i] - min) / range) * (h - 2 * pad);
        points.push(x.toFixed(1) + ',' + y.toFixed(1));
    }

    var last = scores[scores.length - 1];
    var color = last > 0.7 ? 'var(--success)' : (last > 0.3 ? 'var(--warning)' : 'var(--error)');

    return '<svg class="sparkline-svg" viewBox="0 0 ' + w + ' ' + h + '">' +
        '<polyline points="' + points.join(' ') + '" fill="none" stroke="' + color + '" stroke-width="1.5" />' +
        '</svg>';
}

function renderQualityTrends(agentId) {
    var trends = AppState.qualityTrends || [];
    var agentTrends = [];
    for (var i = 0; i < trends.length; i++) {
        if (trends[i].agentId === agentId) {
            agentTrends.push(trends[i]);
        }
    }

    if (agentTrends.length === 0) return '';

    var html = '<div class="quality-trend-section"><div class="category-breakdown-title">Quality Trends</div>';
    for (var j = 0; j < agentTrends.length; j++) {
        html += '<div class="trend-row">' +
            '<span class="trend-label">' + escapeHtml(shortCategory(agentTrends[j].category)) + '</span>' +
            renderSparkline(agentTrends[j].scores) +
            '</div>';
    }
    html += '</div>';
    return html;
}

// Aggregate summary bar — headline learning metrics
function renderSummaryBar() {
    var history = AppState.routingHistory || [];
    var agents = AppState.learningAgents || {};
    var lessons = AppState.lessons || [];

    // Avg quality
    var qualityScores = [];
    for (var i = 0; i < history.length; i++) {
        if (history[i].qualityScore !== undefined && history[i].qualityScore !== null) {
            qualityScores.push(history[i].qualityScore);
        }
    }
    var avgQuality = qualityScores.length > 0
        ? qualityScores.reduce(function(a, b) { return a + b; }, 0) / qualityScores.length
        : null;

    // Quality slope
    var slope = computeSlope(qualityScores);
    var slopeClass = 'slope-flat';
    var slopeChar = '—';
    if (slope > 0.02) { slopeClass = 'slope-up'; slopeChar = '&#x25B2;'; }
    else if (slope < -0.02) { slopeClass = 'slope-down'; slopeChar = '&#x25BC;'; }

    // Exploration rate
    var explorations = 0;
    for (var j = 0; j < history.length; j++) {
        if (history[j].wasExploration) explorations++;
    }
    var explorationPct = history.length > 0 ? (explorations / history.length * 100) : 0;

    // Total observations
    var totalObs = 0;
    var agentIds = Object.keys(agents);
    for (var k = 0; k < agentIds.length; k++) {
        totalObs += (agents[agentIds[k]].totalObservations || 0);
    }

    return '<div class="summary-bar">' +
        '<div class="summary-stat">' +
            '<div class="summary-stat-value">' + (avgQuality !== null ? (avgQuality * 100).toFixed(0) + '%' : '—') + '</div>' +
            '<div class="summary-stat-label">Avg Quality</div>' +
        '</div>' +
        '<div class="summary-stat">' +
            '<div class="summary-stat-value ' + slopeClass + '">' + slopeChar + '</div>' +
            '<div class="summary-stat-label">Trend</div>' +
        '</div>' +
        '<div class="summary-stat">' +
            '<div class="summary-stat-value">' + explorationPct.toFixed(0) + '%</div>' +
            '<div class="summary-stat-label">Exploration</div>' +
        '</div>' +
        '<div class="summary-stat">' +
            '<div class="summary-stat-value">' + lessons.length + '</div>' +
            '<div class="summary-stat-label">Lessons</div>' +
        '</div>' +
        '<div class="summary-stat">' +
            '<div class="summary-stat-value">' + totalObs + '</div>' +
            '<div class="summary-stat-label">Observations</div>' +
        '</div>' +
    '</div>';
}

function computeSlope(values) {
    if (!values || values.length < 3) return 0;
    var n = values.length;
    var sumX = 0, sumY = 0, sumXY = 0, sumX2 = 0;
    for (var i = 0; i < n; i++) {
        sumX += i;
        sumY += values[i];
        sumXY += i * values[i];
        sumX2 += i * i;
    }
    var denom = n * sumX2 - sumX * sumX;
    if (denom === 0) return 0;
    return (n * sumXY - sumX * sumY) / denom;
}

// Quality-over-time chart — multi-agent quality trajectories
function renderQualityChart() {
    var trends = AppState.qualityTrends || [];
    if (trends.length === 0) {
        return '<div class="quality-chart-section">' +
            '<div class="quality-chart-empty">Quality data populates after task completions</div>' +
        '</div>';
    }

    var svgW = 600, svgH = 140, pad = 30, padR = 10, padT = 10;
    var plotW = svgW - pad - padR;
    var plotH = svgH - padT - pad;

    // Find max data points across all agents
    var maxLen = 0;
    for (var t = 0; t < trends.length; t++) {
        if (trends[t].scores.length > maxLen) maxLen = trends[t].scores.length;
    }
    if (maxLen < 1) maxLen = 1;

    // Reference line at 0.5
    var refY = padT + plotH * (1 - 0.5);
    var svg = '<svg class="quality-chart-svg" viewBox="0 0 ' + svgW + ' ' + svgH + '" preserveAspectRatio="none">';

    // Y-axis labels
    svg += '<text x="' + (pad - 4) + '" y="' + (padT + 4) + '" class="chart-axis-label" text-anchor="end">1.0</text>';
    svg += '<text x="' + (pad - 4) + '" y="' + (refY + 4) + '" class="chart-axis-label" text-anchor="end">0.5</text>';
    svg += '<text x="' + (pad - 4) + '" y="' + (padT + plotH + 4) + '" class="chart-axis-label" text-anchor="end">0</text>';

    // Reference line
    svg += '<line x1="' + pad + '" y1="' + refY + '" x2="' + (svgW - padR) + '" y2="' + refY + '" class="quality-ref-line" />';

    // Axis lines
    svg += '<line x1="' + pad + '" y1="' + padT + '" x2="' + pad + '" y2="' + (padT + plotH) + '" stroke="var(--border-color)" stroke-width="1" />';
    svg += '<line x1="' + pad + '" y1="' + (padT + plotH) + '" x2="' + (svgW - padR) + '" y2="' + (padT + plotH) + '" stroke="var(--border-color)" stroke-width="1" />';

    // Draw one polyline per agent trend
    var legendHtml = '<div class="quality-legend">';
    // Use distinct colors per agent
    var palette = ['#6366f1', '#22c55e', '#ef4444', '#eab308', '#06b6d4', '#a855f7', '#f97316', '#ec4899'];

    for (var a = 0; a < trends.length; a++) {
        var scores = trends[a].scores;
        if (scores.length < 1) continue;

        var color = palette[a % palette.length];
        var points = [];
        for (var s = 0; s < scores.length; s++) {
            var x = pad + (s / (maxLen - 1 || 1)) * plotW;
            var y = padT + plotH * (1 - scores[s]);
            points.push(x.toFixed(1) + ',' + y.toFixed(1));
        }
        svg += '<polyline points="' + points.join(' ') + '" fill="none" stroke="' + color + '" stroke-width="2" stroke-linejoin="round" />';

        // End dot
        var lastX = pad + ((scores.length - 1) / (maxLen - 1 || 1)) * plotW;
        var lastY = padT + plotH * (1 - scores[scores.length - 1]);
        svg += '<circle cx="' + lastX.toFixed(1) + '" cy="' + lastY.toFixed(1) + '" r="3" fill="' + color + '" />';

        var agentLabel = shortAgentName(trends[a].agentId);
        var catLabel = shortCategory(trends[a].category);
        var ls = scores[scores.length - 1];
        var dotClass = ls >= 0.75 ? 'dot-good' : (ls >= 0.5 ? 'dot-mid' : 'dot-bad');
        legendHtml += '<span class="quality-legend-item"><span class="quality-legend-dot ' + dotClass + '"></span>' +
            escapeHtml(agentLabel) + (catLabel && catLabel !== 'general' ? ' (' + catLabel + ')' : '') + '</span>';
    }

    svg += '</svg>';
    legendHtml += '</div>';

    return '<div class="quality-chart-section">' + svg + legendHtml + '</div>';
}
