"use strict";

var evidencePaused = null;
var evidenceFilter = '';
var evidenceDisplayed = null;

function renderEvidence(force) {
    var container = document.getElementById('evidence-rows');
    if (!container) return;
    var source = evidencePaused || AppState.telemetry;
    var reading = container.contains(document.activeElement) || container.querySelector('details[open]');
    var deferred = !force && !evidencePaused && evidenceDisplayed && reading && JSON.stringify(evidenceDisplayed) !== JSON.stringify(source);
    var updates = document.getElementById('evidence-updates');
    updates.hidden = !deferred;
    if (!deferred) evidenceDisplayed = source.slice();
    var events = evidenceDisplayed;
    var scope = AppState.route && AppState.route.view === 'telemetry' ? AppState.route : {};
    var visible = events.filter(function(event) {
        return eventMatchesScope(event, scope) && (!evidenceFilter || JSON.stringify(event).toLowerCase().indexOf(evidenceFilter.toLowerCase()) !== -1);
    });
    document.getElementById('evidence-count').textContent = visible.length + ' / ' + events.length + ' events';
    document.getElementById('evidence-retention').textContent = AppState.evictedEvents ? AppState.evictedEvents + ' events evicted · earlier records unavailable' : '';
    var scopeHtml = ['task', 'agent', 'trace'].filter(function(key) { return scope[key]; }).map(function(key) {
        return '<span>' + key[0].toUpperCase() + key.slice(1) + ' · <code>' + escapeHtml(scope[key]) + '</code></span>';
    }).join('');
    if (scopeHtml) scopeHtml += uiHelp('correlation') + routeLink('telemetry', {}, 'All events');
    var scopeElement = document.getElementById('evidence-scope');
    if (scopeElement.innerHTML !== scopeHtml) scopeElement.innerHTML = scopeHtml;
    var fingerprint = JSON.stringify([visible, scope, evidenceFilter]);
    if (container.dataset.fingerprint === fingerprint) return;
    var open = Array.from(container.querySelectorAll('details[open]')).map(function(el) { return el.dataset.eventKey; });
    var html = visible.map(function(event) {
        var type = event.eventType || event.event_type || event.type || 'Event';
        var source = event.agentId || event.agent_id || 'Not reported';
        var key = JSON.stringify([event.timestamp, type, source, event.details]);
        var details = event.details && typeof event.details === 'object' ? event.details : {};
        var task = event.taskId || event.task_id || details.taskId || details.task_id;
        var agent = event.agentId || event.agent_id;
        var links = (task ? routeLink('tasks', { task: task }, 'Inspect task') : '') +
            (agent && agent !== 'system' ? routeLink('agents', { agent: agent }, 'Inspect agent') : '');
        return '<details class="evidence-event" data-event-key="' + escapeHtml(key) + '"' + (open.indexOf(key) !== -1 ? ' open' : '') + '><summary><span class="age">' + escapeHtml(formatTime(event.timestamp) || 'Unknown') + '</span><strong>' + escapeHtml(type) + '</strong><span class="muted">' + escapeHtml(source) + '</span></summary>' +
            (links ? '<div class="inspection-toolbar">' + links + '</div>' : '') + '<pre>' + escapeHtml(JSON.stringify(event, null, 2)) + '</pre></details>';
    }).join('');
    container.innerHTML = html || '<div class="ops-empty">' + (events.length || scopeHtml ? 'No matching events in this session' : 'No events received') + '</div>';
    container.dataset.fingerprint = fingerprint;
}

document.getElementById('evidence-filter').addEventListener('input', function(e) {
    evidenceFilter = e.target.value.trim();
    renderEvidence(true);
});
document.getElementById('evidence-pause').addEventListener('click', function(e) {
    evidencePaused = evidencePaused ? null : AppState.telemetry.slice();
    e.target.textContent = evidencePaused ? 'Resume stream' : 'Pause stream';
    e.target.setAttribute('aria-pressed', String(!!evidencePaused));
    renderEvidence(true);
});
document.getElementById('evidence-updates').addEventListener('click', function() {
    renderEvidence(true);
    var container = document.getElementById('evidence-rows');
    container.tabIndex = -1;
    container.focus({ preventScroll: true });
});
