"use strict";

// Context Topology Matrix - Panel Module
// Hybrid: Swarm Overview (default) + Agent Drill-Down (click agent node)

var contextPollInterval = null;

// Stable agent color palette — hash agent ID to a color
var AGENT_COLORS = ['#4a9eff', '#8b5cf6', '#ec4899', '#f97316', '#14b8a6', '#eab308', '#06b6d4', '#84cc16'];
function agentColor(agentId) {
    var hash = 0;
    for (var i = 0; i < agentId.length; i++) hash = ((hash << 5) - hash + agentId.charCodeAt(i)) | 0;
    return AGENT_COLORS[Math.abs(hash) % AGENT_COLORS.length];
}

function agentTag(agentId) {
    var short = agentId.length > 8 ? agentId.substring(0, 8) : agentId;
    var color = agentColor(agentId);
    return '<span class="context-agent-tag" style="border-color:' + color + ';color:' + color + '">' + escapeHtml(short) + '</span>';
}

function startContextPolling() {
    stopContextPolling();
    contextPollInterval = setInterval(function() {
        if (AppState.activeView === 'context') {
            wsSend({ type: 'requestContextTopology' });
        } else {
            stopContextPolling();
        }
    }, 10000);
}

function stopContextPolling() {
    if (contextPollInterval) {
        clearInterval(contextPollInterval);
        contextPollInterval = null;
    }
}

function handleContextTopologyUpdate(event) {
    AppState.contextTopology = event;
    renderContextPanel();
}

function selectContextAgent(agentId) {
    AppState.selectedContextAgent = (AppState.selectedContextAgent === agentId) ? null : agentId;
    renderContextPanel();
}

function renderContextPanel() {
    var container = document.getElementById('context-container');
    if (!container) return;
    var ctx = AppState.contextTopology;
    if (!ctx) {
        container.innerHTML = '<div class="empty-state"><h3>Waiting for Context Data</h3><p>Submit tasks to populate context topology</p></div>';
        return;
    }

    var sel = AppState.selectedContextAgent;
    var headerSuffix = sel ? '  /  <span class="context-focus-label" style="color:' + agentColor(sel) + '">' + escapeHtml(sel) + '</span>' : '';

    container.innerHTML =
        '<div class="context-header-bar">' +
            '<span class="context-mode-label">' + (sel ? 'Agent Drill-Down' : 'Swarm Overview') + '</span>' +
            headerSuffix +
            (sel ? '<button class="context-deselect-btn" onclick="selectContextAgent(\'' + escapeHtml(sel) + '\')">Back to Swarm</button>' : '') +
        '</div>' +
        '<div class="context-topology-grid">' +
            renderTopZone(ctx, sel) +
            renderLeftZone(ctx, sel) +
            renderCenterZone(ctx, sel) +
            renderRightZone(ctx, sel) +
            renderBottomZone(ctx, sel) +
        '</div>';
}

// --- Center: Agent Mesh with click-to-select ---
function renderCenterZone(ctx, sel) {
    var agents = ctx.agents || [];
    var html = '<div class="context-zone context-center">' +
        '<div class="context-zone-title">Agent Mesh</div>' +
        '<div class="context-agents-grid">';

    if (agents.length === 0) {
        html += '<div class="context-empty-zone">No agents connected</div>';
    } else {
        for (var i = 0; i < agents.length; i++) {
            var a = agents[i];
            var pct = a.contextUtilizationPct || 0;
            var ev = a.expectedValue || 0;
            var color = agentColor(a.agentId);
            var isSelected = sel === a.agentId;
            var isDimmed = sel && !isSelected;
            var shortId = a.agentId.length > 16 ? a.agentId.substring(0, 16) : a.agentId;
            var nodeClass = 'context-agent-node context-agent-clickable' +
                (isSelected ? ' context-agent-selected' : '') +
                (isDimmed ? ' context-agent-dimmed' : '');

            html += '<div class="' + nodeClass + '" onclick="selectContextAgent(\'' + escapeHtml(a.agentId) + '\')"' +
                (isSelected ? ' style="border-color:' + color + ';box-shadow:0 0 12px ' + color + '40"' : '') + '>' +
                renderContextGauge(pct, isSelected ? 72 : 56) +
                '<div class="context-agent-label" style="color:' + color + '">' + escapeHtml(shortId) + '</div>' +
                '<div class="context-agent-meta">' + (a.model ? escapeHtml(a.model) : 'unknown') + '</div>' +
                '<div class="context-agent-stats">' +
                '<span title="Expected value">EV ' + ev.toFixed(2) + '</span>' +
                '<span title="Observations">n=' + (a.totalObservations || 0) + '</span>' +
                '</div>';

            if (a.categoryStats && a.categoryStats.length > 2) {
                html += renderPriorRadar(a.categoryStats);
            }
            html += '</div>';
        }
    }
    html += '</div></div>';
    return html;
}

// --- Top: Strategy Sweep + RLM ---
function renderTopZone(ctx, sel) {
    var rlm = ctx.rlm || {};
    var strategies = ctx.contextStrategies || [];

    var html = '<div class="context-zone context-top">' +
        '<div class="context-top-split">';

    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Strategy Sweep</div>' +
        renderStrategyBars(strategies) +
        '</div>';

    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">RLM Decomposition</div>' +
        '<div class="context-stat-row">' +
        renderStatCard('Manifests', rlm.manifestCount || 0) +
        renderStatCard('Chunks', rlm.totalChunks || 0) +
        renderStatCard('Tokens', formatTokens(rlm.totalTokens || 0)) +
        renderStatCard('Threshold', ((rlm.activationThreshold || 0.7) * 100).toFixed(0) + '%') +
        '</div></div>';

    html += '</div></div>';
    return html;
}

// --- Left: Tool Memory + Lifecycle ---
function renderLeftZone(ctx, sel) {
    var tm = ctx.toolMemory || {};
    var lc = ctx.memoryLifecycle || {};
    var html = '<div class="context-zone context-left">';

    // Tool Memory — in swarm mode show aggregate, in drill-down show agent's window
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Tool Memory' + (sel ? '' : ' (swarm)') + '</div>';

    if (!sel) {
        // Swarm overview: aggregate stats only (no flickering sliding window)
        var totalEntries = tm.entryCount || 0;
        var totalErrors = tm.errorCount || 0;
        var totalDupes = tm.duplicateCount || 0;
        html += '<div class="context-stat-row">' +
            renderStatCard('Active Tools', totalEntries) +
            renderStatCard('Errors', totalErrors) +
            '</div>' +
            '<div class="context-stat-row">' +
            renderStatCard('Duplicates', totalDupes) +
            renderStatCard('Repeat Warn', tm.consecutiveSameType || 0) +
            '</div>';
    } else {
        // Drill-down: full sliding window for this agent
        html += '<div class="context-stat-row">' +
            renderStatCard('Entries', (tm.entryCount || 0) + '/' + (tm.maxEntries || 10)) +
            renderStatCard('Errors', tm.errorCount || 0) +
            '</div>' +
            '<div class="context-stat-row">' +
            renderStatCard('Dupes', tm.duplicateCount || 0) +
            renderStatCard('Repeat', tm.consecutiveSameType || 0) +
            '</div>';

        if (tm.recentActionTypes && tm.recentActionTypes.length > 0) {
            html += '<div class="context-action-list">';
            for (var i = 0; i < Math.min(tm.recentActionTypes.length, 5); i++) {
                html += '<div class="context-action-item">' + escapeHtml(tm.recentActionTypes[i]) + '</div>';
            }
            html += '</div>';
        }
        if (tm.hasObserveData) {
            html += '<div class="context-badge">Observe data cached</div>';
        }
    }
    html += '</div>';

    // Memory Lifecycle
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Memory Lifecycle</div>' +
        '<div class="context-stat-row">' +
        renderStatCard('TTL', (lc.transientTtlDays || 7) + 'd') +
        renderStatCard('Promote', lc.promotionThreshold || 3) +
        '</div>' +
        renderLifecycleFunnel(lc) +
        '</div>';

    html += '</div>';
    return html;
}

// --- Right: Gossip + Lifecycle Transitions ---
function renderRightZone(ctx, sel) {
    var gossip = ctx.gossip || {};
    var lc = ctx.memoryLifecycle || {};
    var html = '<div class="context-zone context-right">';

    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Gossip Network</div>' +
        renderGossipPulse(gossip) +
        '<div class="context-stat-row">' +
        renderStatCard('Episodes', gossip.episodesSynthesized || 0) +
        renderStatCard('Lessons', gossip.lessonsStored || 0) +
        '</div>';
    if (gossip.lastEventSecsAgo != null) {
        html += '<div class="context-meta">Last event ' + gossip.lastEventSecsAgo + 's ago</div>';
    }
    html += '</div>';

    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Lifecycle Transitions</div>' +
        '<div class="context-stat-row">' +
        renderStatCard('Promoted', lc.promoted || 0) +
        renderStatCard('Expired', lc.expired || 0) +
        '</div>' +
        '<div class="context-stat-row">' +
        renderStatCard('Distilled', lc.distilled || 0) +
        renderStatCard('Demoted', lc.demoted || 0) +
        '</div></div>';

    html += '</div>';
    return html;
}

// --- Bottom: Decision Traces + Anti-Patterns (with agent tags in swarm mode) ---
function renderBottomZone(ctx, sel) {
    var traces = ctx.decisionTraces || [];
    var patterns = ctx.antiPatterns || [];

    // In swarm mode, add source agent tags; in drill-down, data is already filtered by backend
    var html = '<div class="context-zone context-bottom">' +
        '<div class="context-bottom-split">';

    // Decision Traces with agent badges
    html += '<div class="context-sub-panel context-sub-wide">' +
        '<div class="context-zone-title">Decision Traces' + (sel ? '' : ' (all agents)') + '</div>' +
        '<div class="context-circuit-scroll">';
    if (!traces || traces.length === 0) {
        html += '<div class="context-empty-zone">No decision traces</div>';
    } else {
        html += renderDecisionCircuitTagged(traces, sel);
    }
    html += '</div></div>';

    // Anti-Pattern Shield with agent badges
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Anti-Pattern Shield</div>' +
        '<div class="context-antipattern-list">';
    if (!patterns || patterns.length === 0) {
        html += '<div class="context-empty-zone">No anti-patterns detected</div>';
    } else {
        html += renderAntiPatternShieldTagged(patterns, sel);
    }
    html += '</div></div>';

    html += '</div></div>';
    return html;
}

// Decision circuit with agent source tags in swarm mode
function renderDecisionCircuitTagged(traces, sel) {
    var w = 560;
    var rowH = 28;
    var h = traces.length * rowH + 8;
    var svg = '<svg width="' + w + '" height="' + h + '" viewBox="0 0 ' + w + ' ' + h + '">';

    for (var i = 0; i < traces.length; i++) {
        var t = traces[i];
        var y = i * rowH + rowH / 2 + 4;
        var reasonColor = t.selectionReason === 'ThompsonSampling' ? 'var(--success)' :
            t.selectionReason === 'BudgetConstrained' ? 'var(--warning)' : 'var(--accent)';

        svg += '<line x1="10" y1="' + y + '" x2="' + (w - 10) + '" y2="' + y + '" ' +
            'stroke="var(--border-color)" stroke-width="1" class="context-flow-pulse"/>';

        svg += '<circle cx="60" cy="' + y + '" r="6" fill="' + reasonColor + '" opacity="0.8"/>';
        svg += '<text x="75" y="' + (y + 4) + '" fill="var(--text-secondary)" font-size="10" font-family="monospace">' +
            escapeHtml(t.taskCategory) + '</text>';

        svg += '<circle cx="240" cy="' + y + '" r="4" fill="' + reasonColor + '"/>';
        svg += '<text x="252" y="' + (y + 4) + '" fill="var(--text-secondary)" font-size="10" font-family="monospace">' +
            escapeHtml(t.selectionReason) + '</text>';

        // Model node with agent color in swarm mode
        var modelColor = (!sel && t.selectedModel) ? agentColor(t.selectedModel) : 'var(--accent)';
        svg += '<circle cx="440" cy="' + y + '" r="6" fill="' + modelColor + '" opacity="0.8"/>';
        var modelShort = t.selectedModel.length > 14 ? t.selectedModel.substring(0, 14) : t.selectedModel;
        svg += '<text x="452" y="' + (y + 4) + '" fill="var(--text-primary)" font-size="10" font-family="monospace">' +
            escapeHtml(modelShort) + '</text>';
    }
    svg += '</svg>';
    return svg;
}

// Anti-pattern shield with agent source tags
function renderAntiPatternShieldTagged(patterns, sel) {
    var html = '';
    for (var i = 0; i < patterns.length; i++) {
        var p = patterns[i];
        var opacity = Math.max(0.3, Math.min(1.0, p.decayedWeight));
        var barWidth = Math.min(100, p.decayedWeight * 100);
        var modelColor = agentColor(p.model);
        html += '<div class="anti-pattern-item" style="opacity:' + opacity + ';border-left-color:' + modelColor + '">' +
            '<div class="anti-pattern-header">' +
            (!sel ? '<span class="context-agent-tag" style="border-color:' + modelColor + ';color:' + modelColor + '">' + escapeHtml(p.model.substring(0, 8)) + '</span> ' : '') +
            '<span class="anti-pattern-sig">' + escapeHtml(p.failureSignature) + '</span>' +
            '<span class="anti-pattern-count">x' + p.failureCount + '</span>' +
            '</div>' +
            '<div class="anti-pattern-bar-track">' +
            '<div class="anti-pattern-bar-fill" style="width:' + barWidth + '%;background:' + modelColor + '"></div>' +
            '</div>' +
            '<div class="anti-pattern-meta">' + escapeHtml(p.category) + '</div>' +
            '</div>';
    }
    return html;
}

// --- Helpers ---
function renderStatCard(label, value) {
    return '<div class="context-stat-card">' +
        '<div class="context-stat-value">' + value + '</div>' +
        '<div class="context-stat-label">' + label + '</div>' +
        '</div>';
}

function formatTokens(n) {
    if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
    return '' + n;
}
