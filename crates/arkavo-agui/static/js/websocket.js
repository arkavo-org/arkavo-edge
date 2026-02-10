"use strict";

var ws = null;
var reconnectAttempts = 0;

function connect() {
    var protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    var wsUrl = protocol + '//' + location.host + '/ws';
    console.log('Connecting to WebSocket:', wsUrl);
    ws = new WebSocket(wsUrl);

    ws.onopen = function() {
        console.log('WebSocket connected');
        setConnectionStatus('connected', 'Connected');
        reconnectAttempts = 0;
        wsSend({ type: 'requestMeshStatus' });
        wsSend({ type: 'requestTaskList' });
    };

    ws.onmessage = function(e) {
        console.log('Received:', e.data);
        var event = JSON.parse(e.data);
        routeEvent(event);
    };

    ws.onerror = function() {
        setConnectionStatus('disconnected', 'Error');
    };

    ws.onclose = function() {
        setConnectionStatus('disconnected', 'Disconnected');
        reconnect();
    };
}

function reconnect() {
    var delay = Math.min(500 * Math.pow(2, reconnectAttempts), 30000);
    reconnectAttempts++;
    setTimeout(connect, delay);
}

function wsSend(obj) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify(obj));
    }
}

function setConnectionStatus(cls, text) {
    var el = document.getElementById('connection-status');
    el.className = cls;
    el.textContent = text;
}
