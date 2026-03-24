"use strict";

// Context Topology Matrix - Panel Module

var contextPollInterval = null;

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

function renderContextPanel() {
    var container = document.getElementById('context-container');
    if (!container) return;
    var ctx = AppState.contextTopology;
    if (!ctx) {
        container.innerHTML = '<div class="empty-state"><h3>Waiting for Context Data</h3><p>Submit tasks to populate context topology</p></div>';
        return;
    }

    container.innerHTML =
        '<div class="context-topology-grid">' +
            renderTopZone(ctx) +
            renderLeftZone(ctx) +
            renderCenterZone(ctx) +
            renderRightZone(ctx) +
            renderBottomZone(ctx) +
        '</div>';
}

// --- Center: Multi-LLM Agent Mesh ---
function renderCenterZone(ctx) {
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
            var nodeColor = ev > 0.7 ? 'var(--success)' : ev > 0.5 ? 'var(--warning)' : 'var(--error)';
            var shortId = a.agentId.length > 16 ? a.agentId.substring(0, 16) : a.agentId;

            html += '<div class="context-agent-node">' +
                renderContextGauge(pct, 56) +
                '<div class="context-agent-label" style="color:' + nodeColor + '">' + escapeHtml(shortId) + '</div>' +
                '<div class="context-agent-meta">' +
                (a.model ? escapeHtml(a.model) : 'unknown') +
                '</div>' +
                '<div class="context-agent-stats">' +
                '<span title="Expected value">EV ' + ev.toFixed(2) + '</span>' +
                '<span title="Observations">n=' + (a.totalObservations || 0) + '</span>' +
                '</div>';

            // Mini radar for category priors
            if (a.categoryStats && a.categoryStats.length > 2) {
                html += renderPriorRadar(a.categoryStats);
            }
            html += '</div>';
        }
    }
    html += '</div></div>';
    return html;
}

// --- Top: Strategy Sweep + RLM Prism ---
function renderTopZone(ctx) {
    var rlm = ctx.rlm || {};
    var strategies = ctx.contextStrategies || [];

    var html = '<div class="context-zone context-top">' +
        '<div class="context-top-split">';

    // Thompson Strategy Gate
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Strategy Sweep</div>' +
        renderStrategyBars(strategies) +
        '</div>';

    // RLM Prism
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">RLM Decomposition</div>' +
        '<div class="context-stat-row">' +
        renderStatCard('Manifests', rlm.manifestCount || 0) +
        renderStatCard('Chunks', rlm.totalChunks || 0) +
        renderStatCard('Tokens', formatTokens(rlm.totalTokens || 0)) +
        renderStatCard('Threshold', ((rlm.activationThreshold || 0.7) * 100).toFixed(0) + '%') +
        '</div>' +
        '</div>';

    html += '</div></div>';
    return html;
}

// --- Left: Tool Memory + Context Ledger ---
function renderLeftZone(ctx) {
    var tm = ctx.toolMemory || {};
    var lc = ctx.memoryLifecycle || {};

    var html = '<div class="context-zone context-left">';

    // Tool Memory Ticker
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Tool Memory</div>' +
        '<div class="context-stat-row">' +
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
    html += '</div>';

    // Context Ledger Vault
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

// --- Right: Gossip + Learning + Priors ---
function renderRightZone(ctx) {
    var gossip = ctx.gossip || {};
    var lc = ctx.memoryLifecycle || {};

    var html = '<div class="context-zone context-right">';

    // Gossip Pulse
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

    // Lifecycle summary
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Lifecycle Transitions</div>' +
        '<div class="context-stat-row">' +
        renderStatCard('Promoted', lc.promoted || 0) +
        renderStatCard('Expired', lc.expired || 0) +
        '</div>' +
        '<div class="context-stat-row">' +
        renderStatCard('Distilled', lc.distilled || 0) +
        renderStatCard('Demoted', lc.demoted || 0) +
        '</div>' +
        '</div>';

    html += '</div>';
    return html;
}

// --- Bottom: Decision Traces + Anti-Patterns ---
function renderBottomZone(ctx) {
    var traces = ctx.decisionTraces || [];
    var patterns = ctx.antiPatterns || [];

    var html = '<div class="context-zone context-bottom">' +
        '<div class="context-bottom-split">';

    // Decision Trace Circuit
    html += '<div class="context-sub-panel context-sub-wide">' +
        '<div class="context-zone-title">Decision Traces</div>' +
        '<div class="context-circuit-scroll">' +
        renderDecisionCircuit(traces) +
        '</div>' +
        '</div>';

    // Anti-Pattern Shield
    html += '<div class="context-sub-panel">' +
        '<div class="context-zone-title">Anti-Pattern Shield</div>' +
        '<div class="context-antipattern-list">' +
        renderAntiPatternShield(patterns) +
        '</div>' +
        '</div>';

    html += '</div></div>';
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
