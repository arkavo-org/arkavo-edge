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

function requestSecurityData() {
    wsSend({ type: 'getSecurityStatus' });
}

function renderSecurity() {
    var container = document.getElementById('security-container');
    if (!container) return;

    var ss = AppState.securityStatus;
    var html = '';

    if (!ss) {
        html = '<div class="empty-state"><h3>Loading Security Status...</h3><p>Requesting KAS and TDF audit data</p></div>';
        container.innerHTML = html;
        return;
    }

    // KAS Status card
    html += '<div class="section-title">KAS Status</div>';
    html += '<div class="stats-grid">';
    html += renderStatCard('KAS', ss.kasEnabled ? 'Enabled' : 'Disabled', '');
    html += renderStatCard('Agent', ss.agentId || 'unknown', '');
    html += renderStatCard('Key ID', ss.keyId || 'none', '');
    html += renderStatCard('Algorithm', ss.encryptionAlgorithm || 'N/A', '');
    html += '</div>';

    // KAS URL
    if (ss.kasUrl) {
        html += '<div class="sys-info">';
        html += '<div class="sys-info-label">KAS Endpoint</div>';
        html += '<div class="sys-info-value mono">' + escapeHtml(ss.kasUrl) + '</div>';
        html += '</div>';
    }

    // Encryption posture
    html += '<div class="section-title">Encryption Posture</div>';
    var auditCount = ss.auditCount || 0;
    html += '<div class="stats-grid">';
    html += renderStatCard('Audit Count', auditCount, '');
    html += renderStatCard('Preflight', ss.preflightEnabled ? 'Active' : 'Inactive', '');
    html += renderStatCard('Policies', ss.preflightPolicies || 0, '');
    html += '</div>';

    // Posture indicator
    var postureClass = ss.kasEnabled ? 'healthy' : 'warning';
    var postureText = ss.kasEnabled ? 'TDF encryption active' : 'KAS not enabled';
    if (ss.preflightEnabled) {
        postureText += ' | Preflight active (' + ss.preflightPolicies + ' policies)';
    }
    html += '<div class="budget-alert ' + postureClass + '">' + escapeHtml(postureText) + '</div>';

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

    container.innerHTML = html;
}

function formatBytes(bytes) {
    if (bytes == null || bytes === 0) return '0 B';
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1048576) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / 1048576).toFixed(1) + ' MB';
}
