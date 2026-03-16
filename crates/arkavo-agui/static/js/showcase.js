"use strict";

// Showcase Mode: Auto-cycling passive monitoring with intelligent view switching.
// Activates via ?showcase query param or the showcase toggle button.
// Uses event priority to jump to relevant views when something interesting happens.

var Showcase = {
    active: false,
    timer: null,
    cycleMs: 12000,
    viewOrder: ['agents', 'tasks', 'learning', 'router', 'health', 'budget', 'telemetry', 'security'],
    currentIndex: 0,
    pausedUntil: 0,
    banner: null,
    bannerTimeout: null,
    eventQueue: [],
    lastEventCounts: { tasks: 0, events: 0, agents: 0 }
};

// Event priority: higher = more interesting, triggers immediate view switch
var EVENT_PRIORITY = {
    agentLost:            { priority: 10, view: 'agents',   label: 'Agent disconnected' },
    agentDiscovered:      { priority: 8,  view: 'agents',   label: 'New agent discovered' },
    budgetAlert:          { priority: 9,  view: 'budget',   label: 'Budget alert' },
    securityStatusUpdate: { priority: 7,  view: 'security', label: 'Security update' },
    tdfAuditEvent:        { priority: 7,  view: 'security', label: 'TDF audit event' },
    lessonExtracted:      { priority: 6,  view: 'learning', label: 'New lesson learned' },
    routingEvaluation:    { priority: 5,  view: 'router',   label: 'Model routing decision' },
    modelSelected:        { priority: 5,  view: 'router',   label: 'Model selected' },
    taskStatusChanged:    { priority: 4,  view: 'tasks',    label: 'Task status changed' },
    taskSubmitted:        { priority: 3,  view: 'tasks',    label: 'New task submitted' },
    systemNotification:   { priority: 6,  view: 'telemetry', label: 'System notification' }
};

// Minimum priority to interrupt the cycle (lower events wait for natural rotation)
var INTERRUPT_THRESHOLD = 6;

function showcaseStart() {
    Showcase.active = true;
    Showcase.currentIndex = 0;
    Showcase.lastEventCounts = {
        tasks: Object.keys(AppState.tasks).length,
        events: AppState.eventCount,
        agents: Object.keys(AppState.agents).length
    };

    document.body.classList.add('showcase-mode');
    createShowcaseBanner();
    createShowcaseOverlay();
    showcaseCycle();

    Showcase.timer = setInterval(function() {
        if (Date.now() < Showcase.pausedUntil) return;
        showcaseCycle();
    }, Showcase.cycleMs);
}

function showcaseStop() {
    Showcase.active = false;
    document.body.classList.remove('showcase-mode');

    if (Showcase.timer) {
        clearInterval(Showcase.timer);
        Showcase.timer = null;
    }

    var banner = document.getElementById('showcase-banner');
    if (banner) banner.remove();

    var overlay = document.getElementById('showcase-overlay');
    if (overlay) overlay.remove();
}

function showcaseToggle() {
    if (Showcase.active) {
        showcaseStop();
    } else {
        showcaseStart();
    }
}

function showcaseCycle() {
    // Check for queued high-priority events first
    if (Showcase.eventQueue.length > 0) {
        var next = Showcase.eventQueue.shift();
        showcaseSwitchTo(next.view, next.label);
        return;
    }

    // Normal rotation
    Showcase.currentIndex = (Showcase.currentIndex + 1) % Showcase.viewOrder.length;
    var viewId = Showcase.viewOrder[Showcase.currentIndex];
    switchView(viewId);
    updateShowcaseOverlay(viewId, null);
}

function showcaseSwitchTo(viewId, reason) {
    // Update index to match the interrupted view
    var idx = Showcase.viewOrder.indexOf(viewId);
    if (idx >= 0) Showcase.currentIndex = idx;

    switchView(viewId);
    updateShowcaseOverlay(viewId, reason);

    // Pause cycle for a bit longer so the user can see the interesting event
    Showcase.pausedUntil = Date.now() + Showcase.cycleMs * 1.5;
}

// Called from routeEvent() in app.js for every incoming WebSocket event
function showcaseOnEvent(event) {
    if (!Showcase.active) return;
    if (!event || !event.type) return;

    var config = EVENT_PRIORITY[event.type];
    if (!config) return;

    if (config.priority >= INTERRUPT_THRESHOLD) {
        // High priority: interrupt the cycle immediately
        showcaseSwitchTo(config.view, config.label);
    } else {
        // Low priority: queue for next natural cycle
        // Avoid duplicate queued views
        var isDup = Showcase.eventQueue.some(function(e) { return e.view === config.view; });
        if (!isDup) {
            Showcase.eventQueue.push({ view: config.view, label: config.label });
        }
    }
}

function createShowcaseBanner() {
    var existing = document.getElementById('showcase-banner');
    if (existing) existing.remove();

    var banner = document.createElement('div');
    banner.id = 'showcase-banner';
    banner.innerHTML = '<span class="showcase-dot"></span> SHOWCASE MODE <span class="showcase-hint">press Esc to exit</span>';
    document.body.appendChild(banner);
}

function createShowcaseOverlay() {
    var existing = document.getElementById('showcase-overlay');
    if (existing) existing.remove();

    var overlay = document.createElement('div');
    overlay.id = 'showcase-overlay';
    overlay.innerHTML = '<div class="showcase-view-label"></div><div class="showcase-reason"></div>';
    document.body.appendChild(overlay);
}

function updateShowcaseOverlay(viewId, reason) {
    var overlay = document.getElementById('showcase-overlay');
    if (!overlay) return;

    var viewNames = {
        agents: 'Agent Mesh',
        tasks: 'Task Queue',
        budget: 'Budget & ROI',
        health: 'Health Monitor',
        security: 'Security & TDF',
        router: 'Router Decisions',
        telemetry: 'Telemetry',
        learning: 'Learning & Models'
    };

    var label = overlay.querySelector('.showcase-view-label');
    var reasonEl = overlay.querySelector('.showcase-reason');

    if (label) label.textContent = viewNames[viewId] || viewId;
    if (reasonEl) {
        if (reason) {
            reasonEl.textContent = reason;
            reasonEl.classList.add('active');
        } else {
            reasonEl.textContent = '';
            reasonEl.classList.remove('active');
        }
    }

    // Brief highlight animation
    overlay.classList.remove('transition');
    void overlay.offsetWidth; // force reflow
    overlay.classList.add('transition');
}

// Keyboard handler: Esc exits showcase mode
document.addEventListener('keydown', function(e) {
    if (e.key === 'Escape' && Showcase.active) {
        showcaseStop();
    }
});

// Auto-activate if ?showcase is in the URL
if (window.location.search.indexOf('showcase') !== -1) {
    // Delay start until DOM and WebSocket are ready
    window.addEventListener('load', function() {
        setTimeout(showcaseStart, 2000);
    });
}
