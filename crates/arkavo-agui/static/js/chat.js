"use strict";

var agentChats = {};

function getAgentChat(agentId) {
    if (!agentChats[agentId]) {
        agentChats[agentId] = { messages: [], isOpen: false };
    }
    return agentChats[agentId];
}

function toggleAgentChat(agentId) {
    var chat = getAgentChat(agentId);
    if (chat.isOpen) {
        closeAgentChat(agentId);
    } else {
        openAgentChat(agentId);
    }
}

function openAgentChat(agentId) {
    var chat = getAgentChat(agentId);
    chat.isOpen = true;

    var area = document.getElementById('chat-area-' + agentId);
    if (area) area.classList.add('active');

    var btn = document.querySelector('[data-chat-toggle="' + agentId + '"]');
    if (btn) btn.classList.add('active');

    wsSend({ type: 'chatOpen', agentId: agentId });

    renderAgentChatMessages(agentId);
    var input = document.getElementById('chat-input-' + agentId);
    if (input) input.focus();
}

function closeAgentChat(agentId) {
    var chat = getAgentChat(agentId);
    chat.isOpen = false;

    var area = document.getElementById('chat-area-' + agentId);
    if (area) area.classList.remove('active');

    var btn = document.querySelector('[data-chat-toggle="' + agentId + '"]');
    if (btn) btn.classList.remove('active');

    wsSend({ type: 'chatClose', agentId: agentId });
}

function handleMessagesSnapshot(event) {
    var agentId = event.agentId || event.agent_id;
    if (!agentId) return;
    var chat = getAgentChat(agentId);
    if (event.messages) {
        chat.messages = event.messages;
        renderAgentChatMessages(agentId);
    }
}

function handleMessageDelta(event) {
    var agentId = event.agentId || event.agent_id;
    if (!agentId) return;

    var chat = getAgentChat(agentId);
    var delta = event.delta;

    // Handle teaching intent metadata
    if (delta && delta.type === 'metadata' && delta.key === 'teaching_intent') {
        var intentData = delta.value || {};
        var intentLabel = intentData.intent || 'unknown';

        // Add a system message showing the detected intent
        chat.messages.push({
            id: Date.now().toString(),
            role: 'system',
            content: '',
            teachingIntent: intentLabel
        });

        // Track teaching event in AppState for the learning panel
        if (!AppState.teachingEvents) AppState.teachingEvents = [];
        AppState.teachingEvents.push({
            intent: intentLabel,
            text: intentData.text || '',
            timestamp: new Date().toISOString()
        });
        while (AppState.teachingEvents.length > 20) {
            AppState.teachingEvents.shift();
        }

        renderAgentChatMessages(agentId);
        return;
    }

    if (delta && delta.type === 'text') {
        var msgId = event.messageId || event.message_id;
        var existingMsg = chat.messages.find(function(m) { return m.id === msgId; });
        if (!existingMsg) {
            existingMsg = { id: msgId, role: 'assistant', content: '' };
            chat.messages.push(existingMsg);
        }
        existingMsg.content += delta.text || '';
        renderAgentChatMessages(agentId);
    }
}

function renderAgentChatMessages(agentId) {
    var container = document.getElementById('chat-msgs-' + agentId);
    if (!container) return;

    var chat = getAgentChat(agentId);
    container.innerHTML = chat.messages.map(function(msg) {
        // Teaching intent system messages get special rendering
        if (msg.teachingIntent) {
            return renderTeachingBadge(msg.teachingIntent);
        }

        var role = escapeHtml(msg.role || 'unknown');
        var roleClass = role.replace(/[^a-zA-Z0-9-]/g, '');
        return '<div class="chat-message ' + roleClass + '">' +
            '<div class="chat-message-role">' + role + '</div>' +
            '<div class="chat-message-content">' + escapeHtml(msg.content) + '</div>' +
            '</div>';
    }).join('');
    container.scrollTop = container.scrollHeight;
}

function renderTeachingBadge(intentLabel) {
    var icon = '';
    var badgeClass = 'teaching-badge';
    if (intentLabel.indexOf('instruction') === 0) {
        icon = '&#x1F4DD;';
        badgeClass += ' instruction';
    } else if (intentLabel === 'correction') {
        icon = '&#x26D4;';
        badgeClass += ' correction';
    } else if (intentLabel === 'reinforcement') {
        icon = '&#x2705;';
        badgeClass += ' reinforcement';
    }

    return '<div class="chat-message system">' +
        '<div class="' + badgeClass + '">' +
            '<span class="teaching-icon">' + icon + '</span>' +
            '<span class="teaching-label">Teaching: ' + escapeHtml(intentLabel) + '</span>' +
        '</div>' +
    '</div>';
}

function sendAgentChatMessage(agentId) {
    var input = document.getElementById('chat-input-' + agentId);
    if (!input) return;
    var content = input.value.trim();
    if (!content) return;

    var chat = getAgentChat(agentId);
    chat.messages.push({ id: Date.now().toString(), role: 'user', content: content });
    renderAgentChatMessages(agentId);

    wsSend({ type: 'userMessage', agentId: agentId, content: content });
    input.value = '';
}

function handleAgentChatKeypress(agentId, event) {
    if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        sendAgentChatMessage(agentId);
    }
}

function handleChatError(event) {
    var msg = event.message || event.code || 'Unknown error';
    showToast(msg, 'error');

    // Show error in the active chat area
    var openChats = Object.keys(agentChats).filter(function(id) {
        return agentChats[id].isOpen;
    });
    openChats.forEach(function(agentId) {
        var chat = getAgentChat(agentId);
        chat.messages.push({
            id: Date.now().toString(),
            role: 'system',
            content: '[Error] ' + msg
        });
        renderAgentChatMessages(agentId);
    });
}
