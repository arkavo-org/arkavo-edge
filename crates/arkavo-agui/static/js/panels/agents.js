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
            '<p>Click an agent to chat</p>' +
            '</div>';
        document.getElementById('agent-count').textContent = '0';
        return;
    }

    container.innerHTML = agents.map(function(agent) {
        var agentId = escapeHtml(agent.id || agent.name);
        var status = escapeHtml(agent.status || 'connected');
        return '<div class="agent-card" onclick="openAgentChat(\'' +
            agentId.replace(/'/g, "\\'") + '\')">' +
            '<div class="agent-header">' +
            '<span class="agent-id">' + (agentId || 'Unknown') + '</span>' +
            '<span class="agent-status ' + status + '">' + status + '</span>' +
            '</div>' +
            '<div class="agent-purpose">' + (escapeHtml(agent.purpose) || 'No purpose specified') + '</div>' +
            '<div class="agent-endpoint">' + (escapeHtml(agent.endpoint) || 'Unknown endpoint') + '</div>' +
            (agent.model ? '<div class="agent-model">' + escapeHtml(agent.model) + '</div>' : '') +
            '</div>';
    }).join('');

    document.getElementById('agent-count').textContent = agents.length;
}
