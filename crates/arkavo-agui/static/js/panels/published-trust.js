"use strict";

// Read-only display of MCP-T trust data the agent publishes externally.
// The composite, dimensions, and behavior.trace events shown here are
// EXACTLY what an external party calling `trust/query` and `trust/history`
// would see. Nothing here gates internal decisions — see the persistent
// banner for the operator-facing explanation.

var PUBLISHED_TRUST_POLL_INTERVAL_MS = 15000;
var publishedTrustPollHandle = null;

// All visible strings on this panel originate from the agent over JSON-RPC.
// A malicious peer could publish a behavior.trace whose subject_id or
// contract_id contains markup, so every interpolated string runs through
// the global escapeHtml() defined in state.js before reaching innerHTML.

function handlePublishedTrustUpdate(event) {
    AppState.lastPublishedTrust = event.snapshot;
    var snap = event.snapshot || {};
    var withoutTs = {};
    for (var k in snap) {
        if (k !== 'fetchedAt' && Object.prototype.hasOwnProperty.call(snap, k)) {
            withoutTs[k] = snap[k];
        }
    }
    var fingerprint = JSON.stringify(withoutTs);
    if (AppState.publishedTrustLastFingerprint === fingerprint) {
        return;
    }
    AppState.publishedTrustLastFingerprint = fingerprint;
    renderPublishedTrust();
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

function pctFidelity(ratio) {
    return Math.round((ratio || 0) * 100);
}

function fidelityBadge(ratio) {
    var pct = pctFidelity(ratio);
    var cls = 'badge-red';
    if (ratio >= 0.95) cls = 'badge-green';
    else if (ratio >= 0.75) cls = 'badge-amber';
    return '<span class="trust-fidelity-badge ' + cls + '">' + pct + '%</span>';
}

function localTime(isoStr) {
    if (!isoStr) return '';
    var d = new Date(isoStr);
    if (isNaN(d.getTime())) return isoStr;
    return d.toLocaleTimeString();
}

function renderScoreCard(score, isSelf) {
    if (!score) return '';
    var dims = score.dimensions || [];
    var dimRows = dims.map(function(d) {
        var bar = Math.min(100, Math.round((d.value / 1000) * 100));
        var conf = Math.round((d.confidence || 0) * 100);
        var muted = (d.confidence || 0) < 0.3 ? ' trust-dim-muted' : '';
        return '' +
            '<div class="trust-dim-row' + muted + '">' +
                '<div class="trust-dim-name">' + escapeHtml(d.name) + '</div>' +
                '<div class="trust-dim-bar-wrap">' +
                    '<div class="trust-dim-bar" style="width:' + bar + '%"></div>' +
                '</div>' +
                '<div class="trust-dim-value">' + d.value + '</div>' +
                '<div class="trust-dim-conf">conf ' + conf + '%</div>' +
                '<div class="trust-dim-evidence">n=' + (d.evidence_count || 0) + '</div>' +
            '</div>';
    }).join('');
    if (!dimRows) {
        dimRows = '<div class="trust-empty-dims">No populated dimensions yet.</div>';
    }

    var sizeCls = isSelf ? 'trust-card trust-card-self' : 'trust-card trust-card-peer';
    return '' +
        '<div class="' + sizeCls + '">' +
            '<div class="trust-card-header">' +
                '<div class="trust-card-subject">' + escapeHtml(score.subjectId) + '</div>' +
                '<div class="trust-card-composite">' + score.composite + '<span class="trust-card-out-of">/1000</span></div>' +
            '</div>' +
            '<div class="trust-dim-grid">' + dimRows + '</div>' +
        '</div>';
}

function renderPublishedTrust() {
    var container = document.getElementById('published-trust-container');
    if (!container) return;

    var snap = AppState.lastPublishedTrust;
    if (!snap) {
        container.innerHTML = '<div class="empty-state"><h3>Loading Published Trust...</h3>' +
            '<p>Querying trust/query and trust/history over the A2A WebSocket</p></div>';
        return;
    }

    var banner = '<div class="trust-advisory-banner">' +
        '<strong>Published externally (advisory only)</strong> &mdash; this score does not gate ' +
        'internal decisions. Each agent acts as its own trust provider; ARP and the router ' +
        'remain the authoritative policy layer.<br>' +
        '<span class="trust-banner-meta">Scored by <code>' + escapeHtml(shortDid(snap.providerDid)) + '</code> ' +
        '&middot; fetched ' + escapeHtml(localTime(snap.fetchedAt)) + '</span>' +
        '</div>';

    var selfSection = '';
    if (snap.selfScore) {
        selfSection = '<div class="trust-section-header">Self-published</div>' +
            renderScoreCard(snap.selfScore, true);
    } else {
        selfSection = '<div class="trust-section-header">Self-published</div>' +
            '<div class="empty-state"><p>No self score yet &mdash; the trust sync writes the local subject ' +
            'on its first 30s tick.</p></div>';
    }

    var peerSection;
    if (snap.peers && snap.peers.length > 0) {
        var peerCards = snap.peers.map(function(p) { return renderScoreCard(p, false); }).join('');
        peerSection = '<div class="trust-section-header">Peer scores (' + snap.peers.length + ')</div>' +
            '<div class="trust-peer-grid">' + peerCards + '</div>';
    } else {
        peerSection = '<div class="trust-section-header">Peer scores</div>' +
            '<div class="empty-state"><p>No peer scores yet &mdash; peers populate as the router ' +
            'observes them via gossip.</p></div>';
    }

    var traceSection;
    if (snap.recentTraces && snap.recentTraces.length > 0) {
        var rows = snap.recentTraces.map(function(t) {
            var contractShort = (t.contractId || '').slice(0, 8);
            return '<tr>' +
                '<td>' + escapeHtml(t.subjectId) + '</td>' +
                '<td><code>' + escapeHtml(contractShort) + '</code></td>' +
                '<td>' + escapeHtml(localTime(t.timestamp)) + '</td>' +
                '<td>' + fidelityBadge(t.fidelityRatio) + '</td>' +
                '<td class="trust-trace-counts">' + t.undeclaredToolCalls + ' / ' + t.totalToolCalls + '</td>' +
                '</tr>';
        }).join('');
        traceSection = '<div class="trust-section-header">Recent behavior.trace events</div>' +
            '<table class="trust-traces-table">' +
                '<thead><tr><th>Subject</th><th>Contract</th><th>Time</th><th>Fidelity</th><th>Undeclared / Total</th></tr></thead>' +
                '<tbody>' + rows + '</tbody>' +
            '</table>';
    } else {
        traceSection = '<div class="trust-section-header">Recent behavior.trace events</div>' +
            '<div class="empty-state"><p>No behavior traces yet &mdash; emitted after each tool-using task.</p></div>';
    }

    container.innerHTML = banner + selfSection + peerSection + traceSection;
}
