"use strict";

function inspectionField(label, value) {
    return '<div><dt>' + label + '</dt><dd>' + value + '</dd></div>';
}

function inspectionSection(key, title, content, open) {
    return '<details class="inspection-section" data-section="' + key + '"' + (open ? ' open' : '') +
        '><summary>' + title + '</summary><div class="inspection-body">' + content + '</div></details>';
}

function updateInspector(container, html, fingerprint) {
    if (container.dataset.detailFingerprint === fingerprint) return;
    var sections = Array.from(container.querySelectorAll('details[data-section]')).map(function(el) {
        return { key: el.dataset.section, open: el.open };
    });
    var active = container.contains(document.activeElement) ? document.activeElement : null;
    var href = active && active.getAttribute('href');
    var section = active && active.closest('details[data-section]');
    container.innerHTML = html;
    container.dataset.detailFingerprint = fingerprint;
    sections.forEach(function(saved) {
        var detail = container.querySelector('[data-section="' + CSS.escape(saved.key) + '"]');
        if (detail) detail.open = saved.open;
    });
    var focus = href ? container.querySelector('a[href="' + CSS.escape(href) + '"]') :
        section ? container.querySelector('[data-section="' + CSS.escape(section.dataset.section) + '"] > summary') : null;
    if (focus) focus.focus({ preventScroll: true });
}

function latestTaskRoute(taskId) {
    for (var i = AppState.routingHistory.length - 1; i >= 0; i--) {
        if (AppState.routingHistory[i].taskId === taskId) return AppState.routingHistory[i];
    }
    return null;
}

function eventMatchesScope(event, scope) {
    var details = event.details && typeof event.details === 'object' ? event.details : {};
    var records = [event, details];
    var fields = {
        task: ['taskId', 'task_id'],
        agent: ['agentId', 'agent_id', 'fromAgent', 'toAgent', 'from_agent', 'to_agent', 'to', 'selectedAgent', 'targetAgent'],
        trace: ['traceId', 'trace_id']
    };
    return Object.keys(fields).every(function(key) {
        return !scope[key] || records.some(function(record) {
            return fields[key].some(function(field) { return record[field] === scope[key]; });
        });
    });
}

function renderMissingRecord(kind, id, view, observation) {
    return routeLink(view, {}, 'All ' + (kind === 'Task' ? 'tasks' : 'agents')) +
        '<div class="ops-empty"><strong>' + kind + ' not in this snapshot</strong><code>' + escapeHtml(id) +
        '</code><p>' + (AppState.observations[observation] ? 'Record unavailable from this gateway.' : 'Awaiting gateway snapshot.') + '</p></div>';
}

function taskBlocker(task) {
    if (task.status === 'input-required') return 'Input required';
    if (task.status === 'auth-required') return 'Authorization required';
    if (task.status === 'failed' || task.status === 'blocked') return task.result ? String(task.result).slice(0, 160) : 'Reason not reported';
    return '';
}
