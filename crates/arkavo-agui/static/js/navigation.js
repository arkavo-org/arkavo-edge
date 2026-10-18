"use strict";

var WORKSPACE_GROUPS = {
    security: [['security', 'Protected data'], ['arp', 'Runtime policy']],
    health: [['health', 'Health'], ['budget', 'Resources'], ['router', 'Routing'], ['learning', 'Adaptation'], ['context', 'Context'], ['diagnostics', 'Diagnostics']]
};

function workspaceFor(viewId) {
    return Object.keys(WORKSPACE_GROUPS).find(function(key) {
        return WORKSPACE_GROUPS[key].some(function(item) { return item[0] === viewId; });
    }) || viewId;
}

function updateNavigation(viewId) {
    var workspace = workspaceFor(viewId);
    document.querySelectorAll('.nav-btn').forEach(function(button) {
        var active = button.dataset.view === workspace;
        button.classList.toggle('active', active);
        if (active) button.setAttribute('aria-current', 'page');
        else button.removeAttribute('aria-current');
    });
    var subnav = document.getElementById('workspace-subnav');
    var header = document.getElementById('view-' + viewId).querySelector('.panel-header');
    if (header) header.after(subnav);
    subnav.innerHTML = (WORKSPACE_GROUPS[workspace] || []).map(function(item) {
        return '<button type="button" data-navigate="' + item[0] + '"' +
            (item[0] === viewId ? ' aria-current="page"' : '') + '>' + item[1] + '</button>';
    }).join('');
    var term = UI_TERMS[workspace];
    document.title = (term ? term[0] + ' · ' : '') + 'Arkavo Operations';
}

function initializeOperations() {
    document.querySelectorAll('[data-nav-term]').forEach(function(button) {
        var term = UI_TERMS[button.dataset.navTerm];
        button.innerHTML = uiIcon(term[1]) + '<span>' + escapeHtml(term[0]) + '</span>';
    });
    document.querySelectorAll('[data-info]').forEach(function(el) { el.innerHTML = uiHelp(el.dataset.info); });
    document.querySelectorAll('[data-create-task], #add-task-btn').forEach(function(button) {
        button.innerHTML = uiIcon('plus') + 'Create task';
    });
    document.getElementById('lexicon-toggle').innerHTML = uiIcon('book-open') + 'Lexicon';
    document.getElementById('lexicon-content').innerHTML = '<dl>' + Object.keys(UI_TERMS).map(function(key) {
        var term = UI_TERMS[key];
        return '<div class="lexicon-entry"><dt>' + uiIcon(term[1]) + escapeHtml(term[0]) + '</dt><dd>' + escapeHtml(term[2]) + '</dd></div>';
    }).join('') + '</dl>';
    document.getElementById('lexicon-toggle').addEventListener('click', function() { document.getElementById('lexicon-dialog').showModal(); });
    document.getElementById('lexicon-close').addEventListener('click', function() { document.getElementById('lexicon-dialog').close(); });
    document.addEventListener('click', function(e) {
        var nav = e.target.closest('[data-navigate]');
        if (nav) switchView(nav.dataset.navigate);
        if (e.target.closest('[data-create-task]')) showAddTaskModal();
    });
    setInterval(function() {
        if (['overview', 'agents', 'tasks', 'decisions'].indexOf(AppState.activeView) !== -1) {
            wsSend({ type: 'requestFleetSnapshot' });
            wsSend({ type: 'requestTaskList' });
        }
        if (['overview', 'decisions'].indexOf(AppState.activeView) !== -1 || AppState.selectedTaskId || AppState.selectedAgentId) wsSend({ type: 'requestArpStatus' });
    }, 5000);
    setInterval(updateObservationAges, 1000);
}
