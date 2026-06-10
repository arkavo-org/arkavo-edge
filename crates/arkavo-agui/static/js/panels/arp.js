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
    // Dedup re-renders: the panel polls every 5s but most poll cycles
    // return identical state. Comparing the serialized snapshot lets us
    // skip the DOM rebuild — preserving <select> focus, table scroll
    // position, and pill hover state — when nothing actually changed.
    // The `timestamp` field is excluded from the comparison since it
    // ticks every poll even when the rest of the snapshot is stable.
    var snap = event.snapshot || {};
    var withoutTs = {};
    for (var k in snap) {
        if (k !== 'timestamp' && Object.prototype.hasOwnProperty.call(snap, k)) {
            withoutTs[k] = snap[k];
        }
    }
    var fingerprint = JSON.stringify(withoutTs);
    if (AppState.arpLastFingerprint === fingerprint) {
        return;
    }
    AppState.arpLastFingerprint = fingerprint;
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
    html += renderSwarmkitLaunchErrors(snap);
    html += renderArpFlightsSection(agents);
    html += renderArpAgentSelector(agents);

    var selected = agents.filter(function(a) { return a.agentId === AppState.arpSelectedAgent; })[0];
    if (!selected) {
        container.innerHTML = html;
        return;
    }

    html += renderArpDocumentSection(selected);
    html += renderArpProposalsSection(selected);
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

    // Pills in the flight section dispatch the same selection action as
    // the dropdown, but route through data-agent-id since the visible
    // label is friendlier than the synthetic agent_id.
    var pills = document.querySelectorAll('.arp-flight-pill');
    Array.prototype.forEach.call(pills, function(pill) {
        pill.addEventListener('click', function() {
            var id = pill.getAttribute('data-agent-id');
            if (!id) return;
            AppState.arpSelectedAgent = id;
            renderArp();
        });
    });

    // Findings-inbox approve/reject buttons. The gateway applies the
    // decision through the proposal queue and answers with a fresh
    // ArpStatusUpdate, so the card re-renders in its new state.
    var proposalBtns = document.querySelectorAll('.arp-proposal-action');
    Array.prototype.forEach.call(proposalBtns, function(btn) {
        btn.addEventListener('click', function(e) {
            e.stopPropagation();
            var pid = btn.getAttribute('data-proposal-id');
            var agentId = btn.getAttribute('data-agent-id');
            var action = btn.getAttribute('data-action');
            if (!pid || !agentId || !action) return;
            btn.disabled = true;
            btn.textContent = action === 'approve' ? 'Approving…' : 'Rejecting…';
            wsSend({
                type: 'requestArpProposalAction',
                agentId: agentId,
                proposalId: pid,
                action: action
            });
        });
    });

    // Stop buttons send a requestStopFlight event; the gateway responds
    // with FlightStopped + an ArpStatusUpdate so the panel re-renders
    // without the dropped flight.
    var stops = document.querySelectorAll('.arp-flight-stop');
    Array.prototype.forEach.call(stops, function(btn) {
        btn.addEventListener('click', function(e) {
            e.stopPropagation();
            var fid = btn.getAttribute('data-flight-id');
            if (!fid) return;
            btn.disabled = true;
            btn.textContent = 'Stopping…';
            wsSend({ type: 'requestStopFlight', flightId: fid });
        });
    });
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
        // SwarmFlight roles carry a flightContext block; render the kit/role
        // metadata instead of the synthetic flight:<uuid>:<role> agent_id.
        var label;
        if (a.flightContext) {
            label = '⚓ ' + a.flightContext.kitName + ' / ' +
                a.flightContext.roleId + ' (' + a.flightContext.roleType + ')';
        } else {
            label = a.agentId;
        }
        html += '<option value="' + escapeHtml(a.agentId) + '"' + selected + '>' +
            escapeHtml(label) + ' (' + status + ')' + vtag + '</option>';
    });
    html += '</select>';
    html += '</td></tr></tbody></table></div>';
    return html;
}

function groupFlightRoles(agents) {
    var byFlight = {};
    var order = [];
    agents.forEach(function(a) {
        if (!a.flightContext) return;
        var fid = a.flightContext.flightId;
        if (!byFlight[fid]) {
            byFlight[fid] = {
                flightId: fid,
                kitId: a.flightContext.kitId,
                kitName: a.flightContext.kitName,
                roles: []
            };
            order.push(fid);
        }
        byFlight[fid].roles.push(a);
    });
    return order.map(function(fid) { return byFlight[fid]; });
}

function renderSwarmkitLaunchErrors(snap) {
    var errors = (snap && snap.swarmkitLaunchErrors) || [];
    if (errors.length === 0) return '';
    var html = '<div class="section-title">SwarmKit auto-launch failed</div>';
    html += '<div class="cost-table"><table><tbody>';
    errors.forEach(function(err) {
        html += '<tr><td>' +
            '<strong>' + escapeHtml(err.kind || 'error') + '</strong>' +
            '<div class="arp-flight-meta"><code>' + escapeHtml(err.path || '') + '</code></div>' +
            '</td><td><span class="status-warn">' + escapeHtml(err.message || '') + '</span></td></tr>';
    });
    html += '</tbody></table></div>';
    return html;
}

function renderArpFlightsSection(agents) {
    var flights = groupFlightRoles(agents);
    if (flights.length === 0) return '';

    var html = '<div class="section-title">Active SwarmFlights</div>';
    html += '<div class="cost-table"><table><tbody>';

    flights.forEach(function(flight) {
        var shortId = (flight.flightId || '').slice(0, 8);
        var rolesCell = '<div class="arp-flight-roles">';

        flight.roles.forEach(function(role) {
            var ctx = role.flightContext;
            var traces = role.decisionTraces || [];
            var traceCount = traces.length;
            var violationCount = traces.filter(isViolation).length;

            var pillClass = 'arp-flight-pill';
            if (role.agentId === AppState.arpSelectedAgent) pillClass += ' arp-flight-pill-selected';
            if (violationCount > 0) pillClass += ' arp-flight-pill-violation';
            else if (!role.documentValid) pillClass += ' arp-flight-pill-invalid';

            var statusGlyph;
            if (violationCount > 0) statusGlyph = '⚠';
            else if (traceCount > 0) statusGlyph = '✓';
            else statusGlyph = '○';

            var counter = '';
            if (violationCount > 0) counter = ' ' + violationCount + ' viol';
            else if (traceCount > 0) counter = ' ' + traceCount;

            // Bound DID — render the orchestrator's role-to-agent
            // assignment so operators can see which mesh agent is
            // playing this role. Empty when the flight is in-process
            // (no real agent behind each role).
            var didLabel = '';
            var titleText = role.agentId;
            if (ctx.boundAgentDid) {
                var did = ctx.boundAgentDid;
                // Truncate did:web:agent-7.example.net → "agent-7" for
                // pill density; full DID still goes into the title.
                var didShort = did;
                var lastColon = did.lastIndexOf(':');
                if (lastColon !== -1 && lastColon < did.length - 1) {
                    didShort = did.slice(lastColon + 1);
                }
                var firstDot = didShort.indexOf('.');
                if (firstDot !== -1) {
                    didShort = didShort.slice(0, firstDot);
                }
                didLabel = ' <span class="arp-flight-pill-agent">@' +
                    escapeHtml(didShort) + '</span>';
                titleText = role.agentId + ' — bound to ' + did;
            }

            rolesCell += '<button type="button" class="' + pillClass + '"' +
                ' data-agent-id="' + escapeHtml(role.agentId) + '"' +
                ' title="' + escapeHtml(titleText) + '">' +
                statusGlyph + ' <strong>' + escapeHtml(ctx.roleId) + '</strong>' +
                ' <span class="arp-flight-pill-type">' + escapeHtml(ctx.roleType) + '</span>' +
                didLabel +
                escapeHtml(counter) +
                '</button>';
        });

        rolesCell += '</div>';

        var stopBtn =
            '<button type="button" class="arp-flight-stop"' +
            ' data-flight-id="' + escapeHtml(flight.flightId) + '"' +
            ' title="Stop this SwarmFlight and remove its roles from the panel">' +
            'Stop</button>';

        html += '<tr><td>' +
            '<strong>' + escapeHtml(flight.kitName) + '</strong> ' + stopBtn +
            '<div class="arp-flight-meta">flight <code>' + escapeHtml(shortId) + '…</code>' +
            ' · ' + flight.roles.length + ' role' + (flight.roles.length === 1 ? '' : 's') + '</div>' +
            '</td><td>' + rolesCell + '</td></tr>';
    });

    html += '</tbody></table></div>';
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

var PROPOSAL_STATE_CLASS = {
    proposed: 'status-warn',
    reviewed: 'status-warn',
    applied: 'status-ok',
    observed: 'status-ok',
    confirmed: 'status-ok',
    reverted: 'status-warn',
    rejected: 'status-warn'
};

function renderArpProposalsSection(snap) {
    var proposals = snap.proposals || [];
    var pending = proposals.filter(function(p) { return p.state === 'proposed'; }).length;
    var html = '<div class="section-title">Findings Inbox' +
        (pending > 0 ? ' <span class="status-warn">(' + pending + ' awaiting review)</span>' : '') +
        '</div>';
    if (proposals.length === 0) {
        html += '<div class="empty-state"><p>No tightening proposals. ' +
            'Audit-plane findings (consolidation teacher, critic, historian) land here for review.</p></div>';
        return html;
    }

    html += '<div class="arp-proposal-cards">';
    proposals.slice().reverse().forEach(function(p) {
        var stateClass = PROPOSAL_STATE_CLASS[p.state] || '';
        html += '<div class="cost-table arp-proposal-card"><table><tbody>';
        html += '<tr><td>Effect</td><td><code>' + escapeHtml(p.effectKind || '') + '</code> ' +
            escapeHtml(p.effectSummary || '') + '</td></tr>';
        html += '<tr><td>Rationale</td><td>' + escapeHtml(p.rationale || '') + '</td></tr>';
        html += '<tr><td>State</td><td><span class="' + stateClass + '">' + escapeHtml(p.state) + '</span>' +
            (p.dispositionReason ? ' — ' + escapeHtml(p.dispositionReason) : '') + '</td></tr>';
        html += '<tr><td>Origin / Radius</td><td><code>' + escapeHtml(p.origin) + '</code> / <code>' +
            escapeHtml(p.blastRadius) + '</code></td></tr>';
        var ev = (p.evidence || []).map(function(t) {
            var eps = (t.episodeIds && t.episodeIds.length) ? ' (' + t.episodeIds.length + ' episodes)' : '';
            return '<code>' + escapeHtml(t.traceId) + '</code>' + eps;
        }).join(', ');
        html += '<tr><td>Evidence</td><td>' + (ev || '--') + '</td></tr>';
        if (p.state === 'proposed') {
            html += '<tr><td>Review</td><td>' +
                '<button type="button" class="arp-proposal-action"' +
                ' data-action="approve" data-proposal-id="' + escapeHtml(p.id) + '"' +
                ' data-agent-id="' + escapeHtml(snap.agentId) + '">Approve</button> ' +
                '<button type="button" class="arp-proposal-action"' +
                ' data-action="reject" data-proposal-id="' + escapeHtml(p.id) + '"' +
                ' data-agent-id="' + escapeHtml(snap.agentId) + '">Reject</button>' +
                '</td></tr>';
        } else if (p.state === 'observed') {
            html += '<tr><td>Review</td><td><span class="status-ok">auto-applied</span> — ' +
                'in observation window; reverts automatically on quality-gate regression</td></tr>';
        } else if (p.reviewedBy) {
            html += '<tr><td>Reviewed by</td><td>' + escapeHtml(p.reviewedBy) + '</td></tr>';
        }
        html += '</tbody></table></div>';
    });
    html += '</div>';
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
        html += '<table class="cost-table"><thead><tr>' +
            '<th>Entity</th><th>Beta(α, β)</th><th>Distribution</th><th>Mean</th>' +
            '<th>Evidence</th><th>Obs</th><th>Health</th>' +
            '</tr></thead><tbody>';
        adaptation.entities.forEach(function(e) {
            html += '<tr>' +
                '<td><code>' + escapeHtml(e.id) + '</code></td>' +
                '<td>' + e.alpha.toFixed(2) + ' / ' + e.beta.toFixed(2) + '</td>' +
                '<td>' + betaCurveSvg(e.alpha, e.beta) + '</td>' +
                '<td>' + e.mean.toFixed(3) + '</td>' +
                '<td>' + evidenceSplitCell(e) + '</td>' +
                '<td>' + e.observations + '</td>' +
                '<td>' + banditHealthCell(e) + '</td>' +
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

// Unnormalized Beta(a,b) density rendered as an inline SVG sparkline.
// Normalization is irrelevant for shape comparison, so the gamma function
// is unnecessary: scale the peak to the viewbox height instead.
function betaCurveSvg(alpha, beta) {
    var W = 90, H = 24, N = 40;
    var pts = [];
    var maxY = 0;
    for (var i = 0; i <= N; i++) {
        var x = (i + 0.5) / (N + 1);
        var y = Math.pow(x, alpha - 1) * Math.pow(1 - x, beta - 1);
        if (!isFinite(y)) y = 0;
        pts.push(y);
        if (y > maxY) maxY = y;
    }
    if (maxY <= 0) maxY = 1;
    var path = '';
    for (var j = 0; j <= N; j++) {
        var px = (j / N) * W;
        var py = H - 2 - (pts[j] / maxY) * (H - 4);
        path += (j === 0 ? 'M' : 'L') + px.toFixed(1) + ',' + py.toFixed(1);
    }
    return '<svg class="arp-beta-curve" width="' + W + '" height="' + H + '"' +
        ' viewBox="0 0 ' + W + ' ' + H + '" aria-label="Beta(' +
        alpha.toFixed(2) + ', ' + beta.toFixed(2) + ')">' +
        '<path d="' + path + '" fill="none" stroke="currentColor" stroke-width="1.5"/>' +
        '</svg>';
}

// Live vs distilled evidence split (prior provenance). Entities without
// distilled mass — or gateways predating provenance — render as all-live.
function evidenceSplitCell(e) {
    if (e.distilledAlpha == null) {
        return '<span title="all evidence is live">live</span>';
    }
    var w = e.distilledWeight != null ? e.distilledWeight : 1.0;
    return 'live ' + e.liveAlpha.toFixed(1) + '/' + e.liveBeta.toFixed(1) +
        ' · <span title="distilled mass × current displacement weight">distilled ' +
        e.distilledAlpha.toFixed(1) + '/' + e.distilledBeta.toFixed(1) +
        ' @ ' + (w * 100).toFixed(0) + '%</span>';
}

// High-variance arms are curiosity targets: the posterior is too wide to
// trust, so the engine (and the operator) should want more samples there.
function banditHealthCell(e) {
    if (e.inWarmup) return '<span class="status-warn">warmup</span>';
    var a = e.alpha, b = e.beta;
    var variance = (a * b) / ((a + b) * (a + b) * (a + b + 1));
    var sd = Math.sqrt(variance);
    if (sd > 0.15) {
        return '<span class="status-warn" title="posterior sd ' + sd.toFixed(3) +
            ' — wide credible interval">curiosity target</span>';
    }
    return '<span class="status-ok" title="posterior sd ' + sd.toFixed(3) + '">converged</span>';
}
