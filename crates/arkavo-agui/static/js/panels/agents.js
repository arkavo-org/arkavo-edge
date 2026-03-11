"use strict";

function handleMeshStatus(event) {
    AppState.agents = {};
    if (event.agents) {
        event.agents.forEach(function(agent) {
            AppState.agents[agent.id || agent.agent_id] = agent;
        });
    }
    renderAgents();
}

function addAgent(event) {
    var id = event.agentId || event.agent_id;
    AppState.agents[id] = {
        id: id,
        endpoint: event.endpoint,
        purpose: event.purpose,
        model: event.model,
        status: 'connected'
    };
    renderAgents();
    addTelemetry({
        eventType: 'agent-connected',
        agentId: id,
        details: { endpoint: event.endpoint },
        timestamp: event.timestamp || new Date().toISOString()
    });
}

function removeAgent(event) {
    var id = event.agentId || event.agent_id;
    if (agentChats[id]) {
        delete agentChats[id];
    }
    delete AppState.agents[id];
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
    var agents = Object.values(AppState.agents);

    if (agents.length === 0) {
        container.innerHTML =
            '<div class="empty-state">' +
            '<h3>No Agents Discovered</h3>' +
            '<p>Agents will appear here when they connect</p>' +
            '</div>';
        document.getElementById('agent-count').textContent = '0';
        return;
    }

    container.innerHTML = agents.map(function(agent) {
        var agentId = escapeHtml(agent.id || agent.name);
        var safeId = agentId.replace(/\\/g, '\\\\').replace(/'/g, "\\'").replace(/"/g, '&quot;');
        var status = escapeHtml(agent.status || 'connected');
        var statusClass = status.replace(/[^a-zA-Z0-9-]/g, '');
        var chat = getAgentChat(agentId);
        var chatActive = chat.isOpen ? ' active' : '';

        return '<div class="agent-card">' +
            '<div class="agent-header">' +
            '<span class="agent-id">' + (agentId || 'Unknown') + '</span>' +
            '<div class="agent-header-actions">' +
            '<span class="agent-status ' + statusClass + '">' + status + '</span>' +
            '<button class="chat-toggle-btn' + chatActive + '" data-chat-toggle="' + safeId + '" title="Chat">&#x2709;</button>' +
            '</div>' +
            '</div>' +
            '<div class="agent-purpose">' + (escapeHtml(agent.purpose) || 'No purpose specified') + '</div>' +
            '<div class="agent-endpoint">' + (escapeHtml(agent.endpoint) || 'Unknown endpoint') + '</div>' +
            (agent.model ? '<div class="agent-model">' + escapeHtml(agent.model) + '</div>' : '') +
            '<div class="agent-chat-area' + chatActive + '" id="chat-area-' + safeId + '">' +
            '<div class="agent-chat-messages" id="chat-msgs-' + safeId + '"></div>' +
            '<div class="agent-chat-input-row">' +
            '<input type="text" class="agent-chat-input" id="chat-input-' + safeId + '" data-agent-id="' + safeId + '" placeholder="Type a message...">' +
            '<button class="agent-chat-send" data-agent-send="' + safeId + '">Send</button>' +
            '</div>' +
            '</div>' +
            '</div>';
    }).join('');

    document.getElementById('agent-count').textContent = agents.length;
    bindAgentChatListeners();

    agents.forEach(function(agent) {
        var agentId = escapeHtml(agent.id || agent.name);
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
