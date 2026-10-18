"use strict";

var ROUTE_FIELDS = {
    overview: [], tasks: ['task'], agents: ['agent'], decisions: ['agent', 'proposal'],
    security: [], telemetry: ['task', 'agent', 'trace'], health: [], budget: [], router: [],
    learning: [], context: [], diagnostics: [], arp: ['agent']
};
var routePositions = new Map();

function parseRoute(hash) {
    var parts = (hash || '').replace(/^#\/?/, '').split('?');
    var view = parts[0] || 'overview';
    if (!Object.prototype.hasOwnProperty.call(ROUTE_FIELDS, view)) return null;
    var route = { view: view };
    var query = new URLSearchParams(parts[1] || '');
    ROUTE_FIELDS[view].forEach(function(key) {
        var value = query.get(key);
        if (value && value.length <= 512 && !/[\u0000-\u001f\u007f]/.test(value)) route[key] = value;
    });
    return route;
}

function routeHref(view, params) {
    if (!Object.prototype.hasOwnProperty.call(ROUTE_FIELDS, view)) return '#/overview';
    var query = new URLSearchParams();
    ROUTE_FIELDS[view].forEach(function(key) {
        if (params && params[key]) query.set(key, params[key]);
    });
    var search = query.toString();
    return '#/' + view + (search ? '?' + search : '');
}

function routeLink(view, params, label, className) {
    return '<a class="' + escapeHtml(className || 'row-action') + '" href="' +
        escapeHtml(routeHref(view, params)) + '">' + escapeHtml(label) + '</a>';
}

function switchView(viewId) {
    navigateRoute(viewId);
}

function navigateRoute(view, params) {
    var href = routeHref(view, params);
    if (location.hash === href) return;
    history.pushState(null, '', href);
    applyRoute(parseRoute(href));
}

function applyRoute(route, initial) {
    if (!route) {
        showToast('View unavailable · showing Overview', 'warning');
        route = { view: 'overview' };
        history.replaceState(null, '', '#/overview');
    }
    var workspace = document.getElementById('workspace');
    var previous = AppState.route;
    if (previous) {
        var active = document.activeElement;
        routePositions.set(routeHref(previous.view, previous), {
            scroll: workspace.scrollTop,
            href: active && active.getAttribute ? active.getAttribute('href') : null,
            filter: evidenceFilter
        });
        if (routePositions.size > 100) routePositions.delete(routePositions.keys().next().value);
    }
    AppState.route = route;
    AppState.selectedTaskId = route.view === 'tasks' ? route.task || null : null;
    AppState.selectedAgentId = route.view === 'agents' ? route.agent || null : null;
    if (route.view === 'arp') AppState.arpSelectedAgent = route.agent || null;
    var position = routePositions.get(routeHref(route.view, route));
    if (route.view === 'telemetry') {
        evidenceFilter = position ? position.filter : '';
        document.getElementById('evidence-filter').value = evidenceFilter;
        evidenceDisplayed = null;
    }
    activateView(route.view);
    workspace.scrollTop = position ? position.scroll : 0;
    if (!initial) {
        var focus = position && position.href ? workspace.querySelector('a[href="' + CSS.escape(position.href) + '"]') : null;
        var header = document.getElementById('view-' + route.view).querySelector('h1');
        if (header) header.tabIndex = -1;
        (focus || header || workspace).focus({ preventScroll: true });
    }
}

function initializeRoutes() {
    // Hashes keep record identifiers local to the browser and work with embedded assets.
    window.addEventListener('hashchange', function() {
        if (location.hash === '#workspace') return;
        var route = parseRoute(location.hash);
        if (route && AppState.route && routeHref(route.view, route) === routeHref(AppState.route.view, AppState.route)) return;
        applyRoute(route);
    });
    if (!location.hash || location.hash === '#workspace') history.replaceState(null, '', '#/overview');
    applyRoute(parseRoute(location.hash), true);
}
