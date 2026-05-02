"use strict";

// Published Trust (MCP-T) — rendered as a section inside the Security &
// Data Plane panel. Trust scoring lives semantically in the security
// posture story, so this file exposes a render function that security.js
// inlines at the bottom of its own output rather than owning a separate
// view.
//
// The displayed score is EXACTLY what an external party calling
// `trust/query` and `trust/history` would see. Nothing here gates
// internal decisions — see the persistent banner.

var PUBLISHED_TRUST_POLL_INTERVAL_MS = 15000;
var publishedTrustPollHandle = null;

// Every visible string on this section originates from the agent over
// JSON-RPC. A malicious peer could publish a behavior.trace whose
// subject_id or contract_id contains markup, so every interpolated
// string runs through the global escapeHtml() defined in state.js.

function handlePublishedTrustUpdate(event) {
    AppState.lastPublishedTrust = event.snapshot;
    // Re-render the security panel only if it's the active view — other
    // panels never display trust data, so a hidden refresh is wasted DOM.
    if (AppState.activeView === 'security' && typeof renderSecurity === 'function') {
        renderSecurity();
    }
}

function startPublishedTrustPolling() {
    if (publishedTrustPollHandle) return;
    publishedTrustPollHandle = setInterval(function() {
        wsSend({ type: 'requestPublishedTrust' });
    }, PUBLISHED_TRUST_POLL_INTERVAL_MS);
}

function stopPublishedTrustPolling() {
    if (publishedTrustPollHandle) {
        clearInterval(publishedTrustPollHandle);
        publishedTrustPollHandle = null;
    }
}

function shortDid(did) {
    if (!did) return '(unknown)';
    if (did.length <= 24) return did;
    return did.slice(0, 16) + '…' + did.slice(-6);
}

function fidelityColor(ratio) {
    if (ratio >= 0.95) return 'var(--success)';
    if (ratio >= 0.75) return 'var(--warning)';
    return 'var(--error)';
}

function dimByName(score, name) {
    if (!score || !score.dimensions) return null;
    for (var i = 0; i < score.dimensions.length; i++) {
        if (score.dimensions[i].name === name) return score.dimensions[i];
    }
    return null;
}

// Returns an HTML fragment that the security panel inlines at the bottom
// of its own output. Reuses budget.js's renderStatCard and the shared
// `section-title` / `cost-table` / `stats-grid` / `budget-alert` classes
// so the trust section visually matches the rest of the panel.
function renderPublishedTrustSection() {
    var html = '<div class="section-title">Published Trust (MCP-T)</div>';

    var snap = AppState.lastPublishedTrust;
    if (!snap) {
        html += '<div class="empty-state"><p>Loading trust data &mdash; ' +
            'querying trust/query and trust/history over the A2A WebSocket.</p></div>';
        return html;
    }

    // Persistent advisory banner — must remain visible so operators don't
    // mistake the displayed score for an internal authorization signal.
    html += '<div class="budget-alert warning">' +
        '<strong>Published externally (advisory only)</strong> &mdash; this score ' +
        'does not gate internal decisions. ARP and the router remain the ' +
        'authoritative policy layer. Scored by <code>' +
        escapeHtml(shortDid(snap.providerDid)) + '</code>.' +
        '</div>';

    // Self-published score: composite + per-dimension stat cards.
    if (snap.selfScore) {
        html += '<div class="section-title">Self-published &mdash; ' +
            escapeHtml(snap.selfSubjectId) + '</div>';
        html += '<div class="stats-grid">';
        html += renderStatCard(
            'Composite',
            snap.selfScore.composite + ' / 1000',
            'fetched ' + formatTime(snap.fetchedAt)
        );
        snap.selfScore.dimensions.forEach(function(d) {
            var conf = Math.round((d.confidence || 0) * 100);
            html += renderStatCard(
                d.name,
                d.value + ' / 1000',
                'conf ' + conf + '% • n=' + (d.evidence_count || 0)
            );
        });
        html += '</div>';
    }

    // Peer trust scores. Single table — one row per peer subject. Until
    // per-peer attestation lands, peers carry only the verification
    // signal; the table makes that explicit so the empty cells are
    // information, not omission.
    html += '<div class="section-title">Peer Trust Scores (' + snap.peers.length + ')</div>';
    if (snap.peers.length === 0) {
        html += '<div class="empty-state">' +
            '<p>No peer scores yet &mdash; peers populate as the router observes ' +
            'them via gossip.</p></div>';
    } else {
        html += '<table class="cost-table"><thead><tr>' +
            '<th>Subject</th><th>Composite</th><th>Verification</th>' +
            '<th>Performance</th><th>Behavioral Fidelity</th>' +
            '</tr></thead><tbody>';
        snap.peers.forEach(function(p) {
            var ver = dimByName(p, 'verification');
            var perf = dimByName(p, 'performance');
            var fid = dimByName(p, 'behavioral_fidelity');
            html += '<tr>' +
                '<td>' + escapeHtml(p.subjectId) + '</td>' +
                '<td class="mono">' + p.composite + '</td>' +
                '<td class="mono">' + (ver ? ver.value : '—') + '</td>' +
                '<td class="mono">' + (perf ? perf.value : '—') + '</td>' +
                '<td class="mono">' + (fid ? fid.value : '—') + '</td>' +
                '</tr>';
        });
        html += '</tbody></table>';
    }

    // Recent behavior.trace events spanning every subject. Subject column
    // tells the operator who emitted the trace (could be self or any
    // peer once cross-agent traces propagate).
    html += '<div class="section-title">Recent behavior.trace Events (' +
        snap.recentTraces.length + ')</div>';
    if (snap.recentTraces.length === 0) {
        html += '<div class="empty-state">' +
            '<p>No behavior traces yet &mdash; emitted after each tool-using task.</p></div>';
    } else {
        html += '<table class="cost-table"><thead><tr>' +
            '<th>Time</th><th>Subject</th><th>Contract</th><th>Fidelity</th>' +
            '<th>Undeclared / Total</th></tr></thead><tbody>';
        snap.recentTraces.forEach(function(t) {
            var pct = Math.round((t.fidelityRatio || 0) * 100);
            var color = fidelityColor(t.fidelityRatio || 0);
            var contractShort = (t.contractId || '').slice(0, 8);
            html += '<tr>' +
                '<td>' + formatTime(t.timestamp) + '</td>' +
                '<td>' + escapeHtml(t.subjectId) + '</td>' +
                '<td class="mono">' + escapeHtml(contractShort) + '</td>' +
                '<td class="mono" style="color:' + color + '">' + pct + '%</td>' +
                '<td class="mono">' + (t.undeclaredToolCalls || 0) +
                ' / ' + (t.totalToolCalls || 0) + '</td>' +
                '</tr>';
        });
        html += '</tbody></table>';
    }

    return html;
}
