"use strict";

function handleSecurityStatusUpdate(event) {
    AppState.securityStatus = event;
    renderSecurity();
}

function handleTdfAuditEvent(event) {
    AppState.tdfAuditLog.unshift(event);
    if (AppState.tdfAuditLog.length > MAX_AUDIT_LOG) {
        AppState.tdfAuditLog.length = MAX_AUDIT_LOG;
    }
    renderSecurity();
}

function handlePolicyApplied(event) {
    AppState.policyLog.unshift(event);
    if (AppState.policyLog.length > MAX_AUDIT_LOG) {
        AppState.policyLog.length = MAX_AUDIT_LOG;
    }
    renderSecurity();
}

function handleDataPlaneStatusUpdate(event) {
    AppState.dataPlaneStatus = event;
    renderSecurity();
}

function handleDataPlaneTransfer(event) {
    AppState.dataPlaneTransfers.unshift(event);
    if (AppState.dataPlaneTransfers.length > MAX_TRANSFERS) {
        AppState.dataPlaneTransfers.length = MAX_TRANSFERS;
    }
    renderSecurity();
}

function requestSecurityData() {
    wsSend({ type: 'getSecurityStatus' });
}

function requestDataPlaneData() {
    wsSend({ type: 'getDataPlaneStatus' });
}

function renderSecurity() {
    var container = document.getElementById('security-container');
    if (!container) return;

    var ss = AppState.securityStatus;
    var dp = AppState.dataPlaneStatus;
    var html = '';

    if (!ss) {
        html = '<div class="empty-state"><h3>Loading Security Status...</h3><p>Requesting KAS and data plane status</p></div>';
        container.innerHTML = html;
        return;
    }

    // Per-agent security posture table
    var agents = ss.agents || [];
    html += '<div class="section-title">Agent Security Posture (' + agents.length + ')</div>';

    if (agents.length === 0) {
        html += '<div class="empty-state"><p>No agents connected</p></div>';
    } else {
        html += '<table class="cost-table"><thead><tr>' +
            '<th>Agent</th><th>KAS</th><th>Key ID</th><th>Algorithm</th><th>Iroh P2P</th>' +
            '</tr></thead><tbody>';
        agents.forEach(function(agent) {
            var kasStyle = agent.kasEnabled
                ? ' style="color:var(--success)"'
                : '';
            var irohStyle = agent.irohActive
                ? ' style="color:var(--success)"'
                : '';
            html += '<tr>' +
                '<td>' + escapeHtml(agent.id) + '</td>' +
                '<td' + kasStyle + '>' + (agent.kasEnabled ? 'Enabled' : '\u2014') + '</td>' +
                '<td class="mono">' + escapeHtml(agent.keyId || '\u2014') + '</td>' +
                '<td class="mono">' + escapeHtml(agent.algorithm || '\u2014') + '</td>' +
                '<td' + irohStyle + '>' + (agent.irohActive ? 'Active' : 'Inactive') + '</td>' +
                '</tr>';
        });
        html += '</tbody></table>';
    }

    // Mesh-wide summary
    var kasCount = agents.filter(function(a) { return a.kasEnabled; }).length;
    var irohCount = agents.filter(function(a) { return a.irohActive; }).length;
    var summaryParts = [];
    if (kasCount > 0) summaryParts.push(kasCount + '/' + agents.length + ' KAS');
    if (irohCount > 0) summaryParts.push(irohCount + '/' + agents.length + ' Iroh');
    var summaryClass = (kasCount > 0 || irohCount > 0) ? 'healthy' : 'warning';
    var summaryText = summaryParts.length > 0
        ? summaryParts.join(' \u2022 ')
        : 'No security services active';
    html += '<div class="budget-alert ' + summaryClass + '">' + escapeHtml(summaryText) + '</div>';

    // Encryption posture
    html += '<div class="section-title">Encryption Posture</div>';
    var auditCount = ss.auditCount || 0;
    html += '<div class="stats-grid">';
    html += renderStatCard('Audit Count', auditCount, '');
    html += renderStatCard('Preflight', ss.preflightEnabled ? 'Active' : 'Inactive', '');
    html += renderStatCard('Policies', ss.preflightPolicies || 0, '');
    html += '</div>';

    // Data Plane aggregate stats
    html += '<div class="section-title">Data Plane (Iroh P2P)</div>';
    if (dp) {
        html += '<div class="stats-grid">';
        html += renderStatCard('Shares Sent', dp.totalSharesSent, formatBytes(dp.totalBytesStaged));
        html += renderStatCard('Shares Received', dp.totalSharesReceived, formatBytes(dp.totalBytesFetched));
        html += renderStatCard('Pending Offers', dp.pendingOffers, '');
        html += '</div>';
    } else {
        html += '<div class="stats-grid">';
        html += renderStatCard('Shares Sent', 0, '');
        html += renderStatCard('Shares Received', 0, '');
        html += renderStatCard('Pending Offers', 0, '');
        html += '</div>';
    }

    // TDF Audit Log
    html += '<div class="section-title">TDF Audit Log (' + AppState.tdfAuditLog.length + ')</div>';
    if (AppState.tdfAuditLog.length === 0) {
        html += '<div class="empty-state"><p>No TDF encryptions recorded yet</p></div>';
    } else {
        html += '<table class="cost-table"><thead><tr>' +
            '<th>Time</th><th>Model</th><th>Msg #</th><th>Algorithm</th><th>Size</th><th>Policies</th>' +
            '</tr></thead><tbody>';
        AppState.tdfAuditLog.forEach(function(entry) {
            html += '<tr>' +
                '<td>' + formatTime(entry.timestamp) + '</td>' +
                '<td>' + escapeHtml(entry.model) + '</td>' +
                '<td class="mono">' + entry.messageIndex + '</td>' +
                '<td class="mono">' + escapeHtml(entry.algorithm) + '</td>' +
                '<td class="mono">' + formatBytes(entry.ciphertextBytes) + '</td>' +
                '<td>' + (entry.policyAttributes ? entry.policyAttributes.length : 0) + '</td>' +
                '</tr>';
        });
        html += '</tbody></table>';
    }

    // Data Plane Activity
    html += renderDataPlaneActivity();

    // Policy Log
    html += '<div class="section-title">Policy Events (' + AppState.policyLog.length + ')</div>';
    if (AppState.policyLog.length === 0) {
        html += '<div class="empty-state"><p>No policy events recorded yet</p></div>';
    } else {
        html += '<table class="cost-table"><thead><tr>' +
            '<th>Time</th><th>Policy</th><th>Action</th><th>Target</th><th>Attributes</th>' +
            '</tr></thead><tbody>';
        AppState.policyLog.forEach(function(entry) {
            html += '<tr>' +
                '<td>' + formatTime(entry.timestamp) + '</td>' +
                '<td>' + escapeHtml(entry.policyId) + '</td>' +
                '<td>' + escapeHtml(entry.action) + '</td>' +
                '<td>' + escapeHtml(entry.target) + '</td>' +
                '<td class="mono">' + entry.attributeCount + '</td>' +
                '</tr>';
        });
        html += '</tbody></table>';
    }

    // Published Trust (MCP-T) — rendered as the bottom section of the
    // Security & Data Plane panel since trust scoring lives in the
    // security posture story rather than its own top-level tab. The
    // function inlines escapeHtml on every interpolated value before
    // assignment to innerHTML below.
    if (typeof renderPublishedTrustSection === 'function') {
        html += renderPublishedTrustSection();
    }

    container.innerHTML = html;
}

function renderDataPlaneActivity() {
    var transfers = AppState.dataPlaneTransfers;
    var html = '';

    html += '<div class="section-title">Data Plane Activity (' + transfers.length + ')</div>';

    if (transfers.length === 0) {
        html += '<div class="empty-state"><p>No P2P transfers recorded yet</p></div>';
        return html;
    }

    html += '<table class="cost-table"><thead><tr>' +
        '<th>Time</th><th>Direction</th><th>Peer</th><th>Hash</th><th>Size</th><th>Status</th>' +
        '</tr></thead><tbody>';
    transfers.forEach(function(entry) {
        var dirIcon = entry.direction === 'sent' ? '\u2191' : '\u2193';
        var statusClass = '';
        if (entry.status === 'failed') statusClass = ' style="color:var(--error)"';
        else if (entry.status === 'staged' || entry.status === 'shared') statusClass = ' style="color:var(--success)"';

        html += '<tr>' +
            '<td>' + formatTime(entry.timestamp) + '</td>' +
            '<td class="mono">' + dirIcon + ' ' + escapeHtml(entry.direction) + '</td>' +
            '<td>' + escapeHtml(entry.peerAgentId) + '</td>' +
            '<td class="mono">' + escapeHtml((entry.contentHash || '').substring(0, 12)) + '</td>' +
            '<td class="mono">' + formatBytes(entry.sizeBytes) + '</td>' +
            '<td' + statusClass + '>' + escapeHtml(entry.status) + '</td>' +
            '</tr>';
    });
    html += '</tbody></table>';

    return html;
}

function formatBytes(bytes) {
    if (bytes == null || bytes === 0) return '0 B';
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1048576) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / 1048576).toFixed(1) + ' MB';
}
