"use strict";

var ARP_POLL_INTERVAL_MS = 5000;
var arpPollHandle = null;

var VIOLATION_EVENT_TYPES = {
    quarantine: true,
    escalation: true,
    budget_event: true,
    hitl_action: true
};

function handleArpStatusUpdate(event) {
    AppState.arpStatus = event.snapshot;
    renderArp();
}

function startArpPolling() {
    if (arpPollHandle) return;
    arpPollHandle = setInterval(function() {
        wsSend({ type: 'requestArpStatus' });
    }, ARP_POLL_INTERVAL_MS);
}

function stopArpPolling() {
    if (arpPollHandle) {
        clearInterval(arpPollHandle);
        arpPollHandle = null;
    }
}

function isViolation(trace) {
    if (trace.success === false) return true;
    if (VIOLATION_EVENT_TYPES[trace.eventType]) return true;
    return false;
}

function renderArp() {
    var container = document.getElementById('arp-container');
    if (!container) return;

    var snap = AppState.arpStatus;
    if (!snap) {
        container.innerHTML = '<div class="empty-state"><h3>Loading ARP Status...</h3><p>Requesting policy document and runtime state</p></div>';
        return;
    }

    var agents = snap.agents || [];
    if (agents.length === 0) {
        container.innerHTML =
            '<div class="empty-state">' +
              '<h3>No Agent Policies</h3>' +
              '<p>Each agent in the mesh owns its own ARP. None loaded yet -- set ARKAVO_ARP_PATH or wait for agents to push policies.</p>' +
            '</div>';
        return;
    }

    if (!AppState.arpSelectedAgent || !agents.some(function(a) { return a.agentId === AppState.arpSelectedAgent; })) {
        AppState.arpSelectedAgent = agents[0].agentId;
    }

    var html = '';
    html += renderArpAgentSelector(agents);

    var selected = agents.filter(function(a) { return a.agentId === AppState.arpSelectedAgent; })[0];
    if (!selected) {
        container.innerHTML = html;
        return;
    }

    html += renderArpDocumentSection(selected);
    html += renderArpViolationsSection(selected.decisionTraces);
    html += renderArpPolicyCacheSection(selected.policyCache);
    html += renderArpAdaptationSection(selected.adaptation);
    html += renderArpTracesSection(selected.decisionTraces);

    container.innerHTML = html;

    var sel = document.getElementById('arp-agent-select');
    if (sel) {
        sel.addEventListener('change', function(e) {
            AppState.arpSelectedAgent = e.target.value;
            renderArp();
        });
    }
}

function renderArpAgentSelector(agents) {
    var violationsByAgent = {};
    agents.forEach(function(a) {
        var traces = a.decisionTraces || [];
        violationsByAgent[a.agentId] = traces.filter(isViolation).length;
    });

    var html = '<div class="section-title">Agent</div>';
    html += '<div class="cost-table"><table><tbody><tr><td>Mesh agents (' + agents.length + ')</td><td>';
    html += '<select id="arp-agent-select">';
    agents.forEach(function(a) {
        var selected = a.agentId === AppState.arpSelectedAgent ? ' selected' : '';
        var status = a.documentValid ? 'OK' : (a.documentPath ? 'INVALID' : 'NONE');
        var vcount = violationsByAgent[a.agentId];
        var vtag = vcount > 0 ? ' [' + vcount + ' viol]' : '';
        html += '<option value="' + escapeHtml(a.agentId) + '"' + selected + '>' +
            escapeHtml(a.agentId) + ' (' + status + ')' + vtag + '</option>';
    });
    html += '</select>';
    html += '</td></tr></tbody></table></div>';
    return html;
}

function renderArpDocumentSection(snap) {
    var html = '<div class="section-title">Document</div>';
    var statusClass = snap.documentValid ? 'status-ok' : 'status-warn';
    var statusLabel = snap.documentValid ? 'VALID' : (snap.documentPath ? 'INVALID' : 'NOT LOADED');

    html += '<div class="cost-table"><table><tbody>';
    html += '<tr><td>Status</td><td><span class="' + statusClass + '">' + escapeHtml(statusLabel) + '</span></td></tr>';
    html += '<tr><td>Path</td><td>' + escapeHtml(snap.documentPath || '(not configured -- set ARKAVO_ARP_PATH)') + '</td></tr>';

    if (snap.loadError) {
        html += '<tr><td>Error</td><td><span class="status-warn">' + escapeHtml(snap.loadError) + '</span></td></tr>';
    }

    var doc = snap.document;
    if (doc) {
        html += '<tr><td>ARP Spec</td><td>' + escapeHtml(doc.arpSpec) + '</td></tr>';
        if (doc.adlUri) {
            html += '<tr><td>ADL URI</td><td>' + escapeHtml(doc.adlUri) + '</td></tr>';
        }
        if (doc.adlHash) {
            html += '<tr><td>ADL Hash</td><td><code>' + escapeHtml(doc.adlHash) + '</code></td></tr>';
        }
        html += '<tr><td>Integrity Signed</td><td>' + (doc.integritySigned ? 'yes' : 'no') + '</td></tr>';
        html += '<tr><td>Adaptation</td><td><code>' + escapeHtml(doc.adaptationMethod) + '</code></td></tr>';
        if (doc.qualityGate) {
            var qg = doc.qualityGate;
            html += '<tr><td>Quality Gate</td><td>threshold ' + qg.threshold +
                ' / metric <code>' + escapeHtml(qg.metric) + '</code>' +
                ' / on-fail <code>' + escapeHtml(qg.onFailure) + '</code></td></tr>';
        }
        if (doc.policyCacheConfig) {
            var pc = doc.policyCacheConfig;
            var hl = pc.halfLifeSec ? ' / half-life ' + pc.halfLifeSec + 's' : '';
            html += '<tr><td>Policy Cache</td><td>TTL ' + pc.ttlSec + 's / decay <code>' +
                escapeHtml(pc.decayStrategy) + '</code>' + hl +
                ' / human-exempt: ' + (pc.humanExempt ? 'yes' : 'no') + '</td></tr>';
        }
        var b = doc.budget;
        var velocity = '';
        if (b.maxSpendPerMinuteUsd != null) velocity += '$' + b.maxSpendPerMinuteUsd + '/min';
        if (b.maxToolCallsPerMinute != null) {
            if (velocity) velocity += ' / ';
            velocity += b.maxToolCallsPerMinute + ' calls/min';
        }
        html += '<tr><td>Budget</td><td>$' + b.taskCeilingUsd + ' / task / on-exhaust <code>' +
            escapeHtml(b.onExhaustion) + '</code>' + (velocity ? ' / ' + velocity : '') + '</td></tr>';

        if (doc.sectionsPresent && doc.sectionsPresent.length) {
            var chips = doc.sectionsPresent.map(function(s) {
                return '<span class="chip">' + escapeHtml(s) + '</span>';
            }).join(' ');
            html += '<tr><td>Sections</td><td>' + chips + '</td></tr>';
        }
    }
    html += '</tbody></table></div>';
    return html;
}

function renderArpViolationsSection(traces) {
    var violations = (traces || []).filter(isViolation);
    var html = '<div class="section-title">Violations <span class="status-warn">(' + violations.length + ')</span></div>';
    if (violations.length === 0) {
        html += '<div class="empty-state"><p>No policy violations recorded. Blocked, denied, or escalated actions appear here.</p></div>';
        return html;
    }
    html += '<table class="cost-table violations-table"><thead><tr>' +
        '<th>Time</th><th>Layer</th><th>Event</th><th>Outcome</th><th>Reason</th><th>Task</th>' +
        '</tr></thead><tbody>';
    violations.slice().reverse().forEach(function(t) {
        var outcome = t.success === false ? 'denied' : 'escalated';
        html += '<tr class="violation-row">' +
            '<td>' + escapeHtml(formatTime(t.timestamp)) + '</td>' +
            '<td><code>' + escapeHtml(t.layer) + '</code></td>' +
            '<td>' + escapeHtml(t.eventType) + '</td>' +
            '<td><span class="status-warn">' + escapeHtml(outcome) + '</span></td>' +
            '<td>' + escapeHtml(t.deniedReason || '--') + '</td>' +
            '<td><code>' + escapeHtml(t.taskId || '--') + '</code></td>' +
            '</tr>';
    });
    html += '</tbody></table>';
    return html;
}

function renderArpPolicyCacheSection(cache) {
    var html = '<div class="section-title">Policy Cache (runtime)</div>';
    if (!cache) {
        html += '<div class="empty-state"><p>No policy loaded for this agent -- runtime cache becomes available after a valid ARP document is loaded.</p></div>';
        return html;
    }
    var chainClass = cache.chainValid ? 'status-ok' : 'status-warn';
    var chainLabel = cache.chainValid ? 'valid' : 'BROKEN';
    html += '<div class="cost-table"><table><tbody>';
    html += '<tr><td>Status</td><td><span class="status-ok">live</span> -- updated by the conductor on every tool outcome</td></tr>';
    html += '<tr><td>Entries</td><td>' + cache.entryCount + '</td></tr>';
    html += '<tr><td>Hash Chain</td><td><span class="' + chainClass + '">' + chainLabel + '</span></td></tr>';
    html += '</tbody></table></div>';

    if (cache.entries && cache.entries.length) {
        html += '<table class="cost-table"><thead><tr><th>Key</th><th>Source</th><th>Influence</th><th>Age</th></tr></thead><tbody>';
        cache.entries.forEach(function(e) {
            html += '<tr>' +
                '<td><code>' + escapeHtml(e.key) + '</code></td>' +
                '<td>' + escapeHtml(e.source) + '</td>' +
                '<td>' + (e.influence != null ? e.influence.toFixed(2) : '--') + '</td>' +
                '<td>' + e.ageSec + 's</td>' +
                '</tr>';
        });
        html += '</tbody></table>';
    } else {
        html += '<div class="empty-state"><p>Cache initialized -- no tool outcomes recorded yet on this agent. Entries populate as the conductor invokes tools.</p></div>';
    }
    return html;
}

function renderArpAdaptationSection(adaptation) {
    var html = '<div class="section-title">Adaptation Engine</div>';
    if (!adaptation) {
        html += '<div class="empty-state"><p>No policy loaded for this agent -- adaptation engine becomes available after a valid ARP document is loaded.</p></div>';
        return html;
    }
    html += '<div class="cost-table"><table><tbody>';
    html += '<tr><td>Status</td><td><span class="status-ok">live</span> -- priors update on every tool outcome</td></tr>';
    html += '<tr><td>Method</td><td><code>' + escapeHtml(adaptation.method) + '</code></td></tr>';
    html += '</tbody></table></div>';

    if (adaptation.entities && adaptation.entities.length) {
        html += '<table class="cost-table"><thead><tr><th>Entity</th><th>alpha</th><th>beta</th><th>Mean</th><th>Obs</th><th>Warmup</th></tr></thead><tbody>';
        adaptation.entities.forEach(function(e) {
            html += '<tr>' +
                '<td><code>' + escapeHtml(e.id) + '</code></td>' +
                '<td>' + e.alpha.toFixed(2) + '</td>' +
                '<td>' + e.beta.toFixed(2) + '</td>' +
                '<td>' + e.mean.toFixed(3) + '</td>' +
                '<td>' + e.observations + '</td>' +
                '<td>' + (e.inWarmup ? 'yes' : 'no') + '</td>' +
                '</tr>';
        });
        html += '</tbody></table>';
    } else {
        html += '<div class="empty-state"><p>Engine initialized -- no priors observed yet on this agent. Entities populate as the conductor selects tools and records outcomes.</p></div>';
    }
    return html;
}

function renderArpTracesSection(traces) {
    var html = '<div class="section-title">Recent Decision Traces</div>';
    if (!traces || !traces.length) {
        html += '<div class="empty-state"><p>No decision traces recorded. Traces appear when the runtime evaluates policies during agent execution.</p></div>';
        return html;
    }
    html += '<table class="cost-table"><thead><tr>' +
        '<th>Time</th><th>Layer</th><th>Event</th><th>Outcome</th><th>Chosen</th><th>Method</th>' +
        '</tr></thead><tbody>';
    traces.slice().reverse().forEach(function(t) {
        var outcomeCell = '--';
        if (t.success === true) outcomeCell = '<span class="status-ok">ok</span>';
        else if (t.success === false) outcomeCell = '<span class="status-warn">denied</span>';
        html += '<tr>' +
            '<td>' + escapeHtml(formatTime(t.timestamp)) + '</td>' +
            '<td><code>' + escapeHtml(t.layer) + '</code></td>' +
            '<td>' + escapeHtml(t.eventType) + '</td>' +
            '<td>' + outcomeCell + '</td>' +
            '<td>' + escapeHtml(t.chosenEntity || '--') + '</td>' +
            '<td>' + escapeHtml(t.selectionMethod || '--') + '</td>' +
            '</tr>';
    });
    html += '</tbody></table>';
    return html;
}
