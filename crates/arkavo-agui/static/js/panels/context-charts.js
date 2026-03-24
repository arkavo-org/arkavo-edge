"use strict";

// Context Topology Matrix - SVG Chart Helpers

function renderContextGauge(pct, size) {
    var r = (size - 6) / 2;
    var cx = size / 2;
    var cy = size / 2;
    var circumference = 2 * Math.PI * r;
    var filled = circumference * Math.min(pct, 100) / 100;
    var color = pct > 85 ? 'var(--error)' : pct > 60 ? 'var(--warning)' : 'var(--accent)';
    return '<svg width="' + size + '" height="' + size + '" viewBox="0 0 ' + size + ' ' + size + '">' +
        '<circle cx="' + cx + '" cy="' + cy + '" r="' + r + '" fill="none" stroke="var(--border-color)" stroke-width="4"/>' +
        '<circle cx="' + cx + '" cy="' + cy + '" r="' + r + '" fill="none" stroke="' + color + '" stroke-width="4" ' +
        'stroke-dasharray="' + filled + ' ' + (circumference - filled) + '" ' +
        'stroke-linecap="round" transform="rotate(-90 ' + cx + ' ' + cy + ')"/>' +
        '<text x="' + cx + '" y="' + (cy + 4) + '" text-anchor="middle" fill="var(--text-primary)" font-size="11" font-family="monospace">' +
        Math.round(pct) + '%</text></svg>';
}

function renderStrategyBars(strategies) {
    if (!strategies || strategies.length === 0) {
        return '<div class="context-empty-zone">No strategy data</div>';
    }
    var maxScore = 0;
    for (var i = 0; i < strategies.length; i++) {
        if (strategies[i].compositeScore > maxScore) maxScore = strategies[i].compositeScore;
    }
    if (maxScore === 0) maxScore = 1;

    var html = '';
    var colors = ['var(--accent)', '#8b5cf6', 'var(--success)', 'var(--warning)', '#ec4899'];
    for (var j = 0; j < strategies.length; j++) {
        var s = strategies[j];
        var widthPct = (s.compositeScore / maxScore) * 100;
        var isActive = s.compositeScore === maxScore && s.burstCount > 0;
        html += '<div class="strategy-bar' + (isActive ? ' strategy-active' : '') + '">' +
            '<span class="strategy-label">' + escapeHtml(s.strategy) + '</span>' +
            '<div class="strategy-track">' +
            '<div class="strategy-fill" style="width:' + widthPct + '%;background:' + colors[j % colors.length] + '"></div>' +
            '</div>' +
            '<span class="strategy-score">' + s.compositeScore.toFixed(2) + '</span>' +
            '</div>';
    }
    return html;
}

function renderLifecycleFunnel(lc) {
    var w = 200;
    var h = 140;
    var svg = '<svg width="' + w + '" height="' + h + '" viewBox="0 0 ' + w + ' ' + h + '">';

    var tiers = [
        { label: 'Transient', count: lc.promoted + lc.expired, color: 'var(--text-secondary)', width: 160 },
        { label: 'Candidate', count: lc.promoted, color: 'var(--accent)', width: 120 },
        { label: 'Canonical', count: lc.distilled, color: 'var(--success)', width: 80 }
    ];
    var tierH = 36;
    for (var i = 0; i < tiers.length; i++) {
        var t = tiers[i];
        var x = (w - t.width) / 2;
        var y = i * tierH + 8;
        svg += '<rect x="' + x + '" y="' + y + '" width="' + t.width + '" height="' + (tierH - 6) + '" ' +
            'rx="4" fill="' + t.color + '" opacity="0.2" stroke="' + t.color + '" stroke-width="1"/>';
        svg += '<text x="' + (w / 2) + '" y="' + (y + 14) + '" text-anchor="middle" ' +
            'fill="var(--text-primary)" font-size="10" font-family="monospace">' + t.label + '</text>';
        svg += '<text x="' + (w / 2) + '" y="' + (y + 26) + '" text-anchor="middle" ' +
            'fill="' + t.color + '" font-size="11" font-weight="bold" font-family="monospace">' + t.count + '</text>';
    }

    // Funnel connecting lines
    for (var j = 0; j < 2; j++) {
        var topW = tiers[j].width;
        var botW = tiers[j + 1].width;
        var ty = j * tierH + tierH + 2;
        var by = ty + 6;
        svg += '<line x1="' + ((w - topW) / 2 + 10) + '" y1="' + ty + '" x2="' + ((w - botW) / 2 + 10) + '" y2="' + by + '" stroke="var(--border-color)" stroke-width="1"/>';
        svg += '<line x1="' + ((w + topW) / 2 - 10) + '" y1="' + ty + '" x2="' + ((w + botW) / 2 - 10) + '" y2="' + by + '" stroke="var(--border-color)" stroke-width="1"/>';
    }

    svg += '</svg>';
    return svg;
}

function renderGossipPulse(stats) {
    var w = 180;
    var h = 40;
    var svg = '<svg width="' + w + '" height="' + h + '" viewBox="0 0 ' + w + ' ' + h + '">';
    var cx = w / 2;
    var cy = h / 2;

    // Pulse rings
    for (var i = 0; i < 3; i++) {
        var r = 8 + i * 10;
        var opacity = 0.6 - i * 0.15;
        svg += '<circle cx="' + cx + '" cy="' + cy + '" r="' + r + '" fill="none" ' +
            'stroke="var(--accent)" stroke-width="1" opacity="' + opacity + '">' +
            '<animate attributeName="r" from="' + r + '" to="' + (r + 8) + '" dur="2s" begin="' + (i * 0.4) + 's" repeatCount="indefinite"/>' +
            '<animate attributeName="opacity" from="' + opacity + '" to="0" dur="2s" begin="' + (i * 0.4) + 's" repeatCount="indefinite"/>' +
            '</circle>';
    }

    // Center dot
    svg += '<circle cx="' + cx + '" cy="' + cy + '" r="4" fill="var(--accent)"/>';

    // Peer count labels
    svg += '<text x="8" y="14" fill="var(--text-secondary)" font-size="9" font-family="monospace">peers</text>';
    svg += '<text x="8" y="28" fill="var(--text-primary)" font-size="14" font-weight="bold" font-family="monospace">' + stats.gossipPeers + '</text>';
    svg += '<text x="' + (w - 8) + '" y="14" text-anchor="end" fill="var(--text-secondary)" font-size="9" font-family="monospace">events</text>';
    svg += '<text x="' + (w - 8) + '" y="28" text-anchor="end" fill="var(--text-primary)" font-size="14" font-weight="bold" font-family="monospace">' + stats.eventsReceived + '</text>';

    svg += '</svg>';
    return svg;
}

function renderPriorRadar(categoryStats) {
    if (!categoryStats || categoryStats.length === 0) return '';
    var size = 120;
    var cx = size / 2;
    var cy = size / 2;
    var r = 45;
    var n = Math.min(categoryStats.length, 8);

    var svg = '<svg width="' + size + '" height="' + size + '" viewBox="0 0 ' + size + ' ' + size + '">';

    // Background rings
    for (var ring = 1; ring <= 3; ring++) {
        svg += '<circle cx="' + cx + '" cy="' + cy + '" r="' + (r * ring / 3) + '" fill="none" stroke="var(--border-color)" stroke-width="0.5"/>';
    }

    // Axes and data polygon
    var points = '';
    for (var i = 0; i < n; i++) {
        var angle = (2 * Math.PI * i / n) - Math.PI / 2;
        var axX = cx + r * Math.cos(angle);
        var axY = cy + r * Math.sin(angle);
        svg += '<line x1="' + cx + '" y1="' + cy + '" x2="' + axX + '" y2="' + axY + '" stroke="var(--border-color)" stroke-width="0.5"/>';

        var ev = categoryStats[i].expectedValue;
        var dataR = r * ev;
        var dx = cx + dataR * Math.cos(angle);
        var dy = cy + dataR * Math.sin(angle);
        points += dx + ',' + dy + ' ';

        // Category label
        var labelX = cx + (r + 12) * Math.cos(angle);
        var labelY = cy + (r + 12) * Math.sin(angle);
        var short = categoryStats[i].category.length > 6 ? categoryStats[i].category.substring(0, 6) : categoryStats[i].category;
        svg += '<text x="' + labelX + '" y="' + (labelY + 3) + '" text-anchor="middle" fill="var(--text-secondary)" font-size="7" font-family="monospace">' + escapeHtml(short) + '</text>';
    }

    if (points) {
        svg += '<polygon points="' + points.trim() + '" fill="var(--accent)" fill-opacity="0.2" stroke="var(--accent)" stroke-width="1.5"/>';
    }

    svg += '</svg>';
    return svg;
}
