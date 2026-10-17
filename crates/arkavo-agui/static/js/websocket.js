"use strict";

var ws = null;
var reconnectAttempts = 0;

function connect() {
    var protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    var wsUrl = protocol + '//' + location.host + '/ws';
    ws = new WebSocket(wsUrl);

    ws.onopen = function() {
        setConnectionStatus('connected', 'Connected');
        reconnectAttempts = 0;
        wsSend({ type: 'requestFleetSnapshot' });
        wsSend({ type: 'requestTaskList' });
        activateView(AppState.activeView);
    };

    ws.onmessage = function(e) {
        var event;
        try { event = JSON.parse(e.data); } catch (_) { return; }
        if (!event || typeof event !== 'object' || typeof event.type !== 'string') return;
        routeEvent(event);
    };

    ws.onerror = function() {
        setConnectionStatus('disconnected', 'Disconnected');
    };

    ws.onclose = function() {
        setConnectionStatus('disconnected', 'Disconnected');
        reconnect();
    };
}

function reconnect() {
    var delay = Math.min(500 * Math.pow(2, reconnectAttempts), 30000);
    reconnectAttempts++;
    setConnectionStatus('reconnecting', 'Reconnecting');
    setTimeout(connect, delay);
}

function wsSend(obj) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        try {
            ws.send(JSON.stringify(obj));
            return true;
        } catch (_) { return false; }
    }
    return false;
}

function setConnectionStatus(cls, text) {
    AppState.connection = cls;
    var el = document.getElementById('connection-status');
    el.className = cls;
    el.innerHTML = 'Gateway ' + uiState(cls);
    var notice = document.getElementById('connection-notice');
    notice.hidden = cls === 'connected';
    notice.textContent = 'Gateway ' + text.toLowerCase() + ' · showing last received data';
    if (cls !== 'connected') {
        markSubmissionUnknown();
        if (AppState.reviewPending) AppState.reviewResult = 'Outcome unknown · gateway disconnected before a review receipt';
    }
    updateComposerAvailability();
    if (AppState.activeView === 'overview') renderOverview();
    if (AppState.activeView === 'decisions') renderDecisions();
}
