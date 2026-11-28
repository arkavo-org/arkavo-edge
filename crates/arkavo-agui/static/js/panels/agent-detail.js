"use strict";

// What the agent's signed card proves (SRV-017): the key that speaks for the
// agent, never where the key is kept, which a card cannot show.
function agentIdentity(identity) {
    if (!identity) return 'Not reported';
    if (identity.state === 'verified') {
        return '<code>' + escapeHtml(identity.did) + '</code> <span class="muted">Card signed by this key</span>';
    }
    if (identity.state === 'unsigned') return 'Card not signed';
    return '<span class="state state-bad">' + uiIcon('ban') + 'Card signature does not verify</span> ' +
        escapeHtml(identity.reason || '');
}

var KEY_PURPOSES = { gossip: 'Gossip', iroh: 'iroh endpoint', tdf: 'TDF' };

// The keys the agent's identity vouches for in its card, as the gateway
// checked them.
function agentKeys(identity) {
    if (!identity || identity.state !== 'verified') return 'Not reported';
    var keys = identity.keys || [];
    if (!keys.length) return 'None vouched for';
    return keys.map(function(key) {
        return '<div>' + escapeHtml(KEY_PURPOSES[key.purpose] || key.purpose) +
            ' <code>' + escapeHtml(key.key) + '</code></div>';
    }).join('');
}

function renderAgentDetail() {
    var container = document.getElementById('agent-inspector');
    var id = AppState.selectedAgentId;
    if (!id) {
        container.innerHTML = '';
        delete container.dataset.detailFingerprint;
        return;
    }
    var agent = AppState.agents[id];
    if (!agent) {
        updateInspector(container, renderMissingRecord('Agent', id, 'agents', 'swarm'),
            JSON.stringify([id, !!AppState.observations.swarm]));
        return;
    }
    var tasks = sortedTasks().filter(function(task) { return getTaskAgent(task, task.id) === id; });
    var policy = ((AppState.arpStatus && AppState.arpStatus.agents) || []).find(function(item) { return item.agentId === id; });
    var proposals = policy ? (policy.proposals || []).filter(function(item) { return item.state === 'proposed'; }) : [];
    var html = '<div class="inspection-toolbar">' + routeLink('agents', {}, 'All agents') +
        routeLink('arp', { agent: id }, 'Runtime policy') + routeLink('telemetry', { agent: id }, 'Agent evidence') + '</div>';
    html += '<section class="ops-section"><div class="ops-section-heading"><h2>Agent <code>' + escapeHtml(id) + '</code></h2>' + uiState(agent.status) + '</div>';
    html += '<dl class="inspection-fields">' + inspectionField('Model', escapeHtml(agent.model || 'Not reported')) +
        inspectionField('Endpoint', escapeHtml(agent.endpoint || 'Not reported')) +
        inspectionField('Identity', agentIdentity(agent.identity)) +
        inspectionField('Keys', agentKeys(agent.identity)) +
        inspectionField('Authorization ' + uiHelp('grant'), 'Not reported') +
        inspectionField('Snapshot ' + uiHelp('freshness'), receivedAge('swarm')) + '</dl></section>';
    html += '<section class="ops-section"><div class="ops-section-heading"><h2>Assigned tasks</h2><span class="muted">' + receivedAge('tasks') + '</span></div>' +
        (tasks.length ? taskRows(tasks) : '<div class="ops-empty">' + (AppState.observations.tasks ? 'No assigned tasks reported' : 'Tasks not reported') + '</div>') + '</section>';
    html += '<section class="ops-section"><div class="ops-section-heading"><h2>Needs review</h2><span class="muted">' + receivedAge('policy') + '</span></div>';
    html += proposals.length ? proposals.map(function(proposal) {
        return '<div class="observation-row"><span>' + escapeHtml(proposal.effectSummary || 'Policy change') + '</span>' +
            routeLink('decisions', { agent: id, proposal: proposal.id }, 'Review') + '</div>';
    }).join('') : '<div class="ops-empty">' + (policy ? 'No pending policy changes' : 'Policy not reported') + '</div>';
    html += '</section>';
    updateInspector(container, html, JSON.stringify([id, agent, tasks, proposals, !!policy,
        !!AppState.observations.tasks, !!AppState.observations.swarm, !!AppState.observations.policy]));
}
