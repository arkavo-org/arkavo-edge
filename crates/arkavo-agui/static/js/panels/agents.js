"use strict";

function handleMeshStatus(event) {
    var current = Object.create(null);
    (event.agents || []).forEach(function(agent) {
        var id = safeTaskKey(agent.id || agent.agent_id);
        if (id) current[id] = Object.assign({}, agent, { id: id, status: agent.status || 'unknown' });
    });
    Object.keys(AppState.agents).forEach(function(id) {
        if (!current[id]) current[id] = Object.assign({}, AppState.agents[id], { status: 'disconnected' });
    });
    AppState.agents = current;
    renderAgents();
}

function addAgent(event) {
    var id = safeTaskKey(event.agentId || event.agent_id);
    if (!id) return;
    AppState.agents[id] = {
        id: id,
        endpoint: event.endpoint,
        purpose: event.purpose,
        model: event.model,
        status: 'unknown'
    };
    renderAgents();
    addTelemetry({
        eventType: 'agent-discovered',
        agentId: id,
        details: { endpoint: event.endpoint },
        timestamp: event.timestamp || new Date().toISOString()
    });
}

function removeAgent(event) {
    var id = safeTaskKey(event.agentId || event.agent_id);
    if (!id) return;
    if (AppState.agents[id]) AppState.agents[id].status = 'disconnected';
    renderAgents();
    addTelemetry({
        eventType: 'agent-disconnected',
        agentId: id,
        details: { reason: event.reason },
        timestamp: event.timestamp || new Date().toISOString()
    });
}

function renderAgents() {
    var container = document.getElementById('agents-container');
    if (!container) return;
    var agents = Object.values(AppState.agents).filter(function(agent) { return !AppState.selectedAgentId || agent.id === AppState.selectedAgentId; });
    var fingerprint = JSON.stringify([AppState.selectedAgentId, agents]);
    if (container.dataset.fingerprint === fingerprint) return;
    container.dataset.fingerprint = fingerprint;
    container.querySelectorAll('.agent-chat-input').forEach(function(input) { getAgentChat(input.dataset.agentId).draft = input.value; });
    var focusedId = container.contains(document.activeElement) ? document.activeElement.id : null;


    if (agents.length === 0) {
        if (AppState.selectedAgentId) { container.innerHTML = ''; return; }
        container.innerHTML =
            '<div class="empty-state">' +
            '<h3>No agents discovered</h3>' +
            '<p>Awaiting agent discovery</p>' +
            '</div>';
        document.getElementById('agent-count').textContent = '0';
        return;
    }

    container.innerHTML = agents.map(function(agent) {
        var agentId = agent.id || agent.name;
        var safeId = escapeHtml(agentId);
        var chat = getAgentChat(agentId);
        var chatActive = chat.isOpen ? ' active' : '';

        return '<div class="agent-card">' +
            '<div class="agent-header">' +
            routeLink('agents', { agent: agentId }, agentId || 'Unknown', 'agent-id') +
            '<div class="agent-header-actions">' +
            uiState(agent.status) +
            '<button class="chat-toggle-btn' + chatActive + '" data-chat-toggle="' + safeId + '" aria-label="Chat with ' + safeId + '">' + uiIcon('message-square') + ' Chat</button>' +
            '</div>' +
            '</div>' +
            '<div class="agent-purpose">' + (escapeHtml(agent.purpose) || 'Purpose not reported') + '</div>' +
            '<div class="agent-endpoint">' + (escapeHtml(agent.endpoint) || 'Endpoint not reported') + '</div>' +
            (agent.model ? '<div class="agent-model">' + escapeHtml(agent.model) + '</div>' : '') +
            '<div class="agent-chat-area' + chatActive + '" id="chat-area-' + safeId + '">' +
            '<div class="agent-chat-messages" id="chat-msgs-' + safeId + '"></div>' +
            '<div class="agent-chat-input-row">' +
            '<input type="text" class="agent-chat-input" id="chat-input-' + safeId + '" data-agent-id="' + safeId + '" aria-label="Message agent" placeholder="Message agent">' +
            '<button class="agent-chat-send" data-agent-send="' + safeId + '">Send</button>' +
            '</div>' +
            '</div>' +
            '</div>';
    }).join('');

    document.getElementById('agent-count').textContent = Object.keys(AppState.agents).length;
    bindAgentChatListeners();
    container.querySelectorAll('.agent-chat-input').forEach(function(input) { input.value = getAgentChat(input.dataset.agentId).draft || ''; });
    if (focusedId) {
        var focused = document.getElementById(focusedId);
        if (focused) focused.focus({ preventScroll: true });
    }

    agents.forEach(function(agent) {
        var agentId = agent.id || agent.name;
        var chat = getAgentChat(agentId);
        if (chat.isOpen && chat.messages.length > 0) {
            renderAgentChatMessages(agentId);
        }
    });
}

function bindAgentChatListeners() {
    var toggleBtns = document.querySelectorAll('[data-chat-toggle]');
    for (var i = 0; i < toggleBtns.length; i++) {
        (function(btn) {
            btn.addEventListener('click', function(e) {
                e.stopPropagation();
                toggleAgentChat(btn.getAttribute('data-chat-toggle'));
            });
        })(toggleBtns[i]);
    }

    var sendBtns = document.querySelectorAll('[data-agent-send]');
    for (var j = 0; j < sendBtns.length; j++) {
        (function(btn) {
            btn.addEventListener('click', function() {
                sendAgentChatMessage(btn.getAttribute('data-agent-send'));
            });
        })(sendBtns[j]);
    }

    var inputs = document.querySelectorAll('.agent-chat-input');
    for (var k = 0; k < inputs.length; k++) {
        (function(input) {
            input.addEventListener('keypress', function(e) {
                handleAgentChatKeypress(input.getAttribute('data-agent-id'), e);
            });
        })(inputs[k]);
    }
}
