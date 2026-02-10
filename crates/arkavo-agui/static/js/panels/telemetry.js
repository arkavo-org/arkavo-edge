"use strict";

function addA2AMessage(event) {
    addTelemetry({
        eventType: 'a2a-message',
        agentId: event.fromAgent || event.from_agent,
        details: { to: event.toAgent || event.to_agent, method: event.method },
        timestamp: event.timestamp || new Date().toISOString()
    });
}

function addTelemetry(event) {
    AppState.telemetry.unshift(event);
    if (AppState.telemetry.length > MAX_TELEMETRY) {
        AppState.telemetry.pop();
    }
    AppState.eventCount++;
    renderTelemetry();
    updateStats();
}

function renderTelemetry() {
    var container = document.getElementById('telemetry-container');
    if (!container) return;

    if (AppState.telemetry.length === 0) {
        container.innerHTML =
            '<div class="empty-state">' +
            '<h3>Waiting for Events</h3>' +
            '<p>Telemetry events will stream here</p>' +
            '</div>';
        return;
    }

    container.innerHTML = AppState.telemetry.map(function(evt) {
        var eventType = evt.eventType || evt.event_type || '';
        var typeClass = eventType.replace(/_/g, '-').replace(/[^a-zA-Z0-9-]/g, '');
        return '<div class="telemetry-item ' + typeClass + '">' +
            '<div class="telemetry-time">' + escapeHtml(formatTime(evt.timestamp)) + '</div>' +
            '<div class="telemetry-type">' + (escapeHtml(eventType) || 'event') + '</div>' +
            '<div class="telemetry-details">' +
            escapeHtml(evt.agentId || evt.agent_id || '') +
            (evt.details ? ' - ' + escapeHtml(JSON.stringify(evt.details).slice(0, 60)) : '') +
            '</div></div>';
    }).join('');
}
