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
        var role = escapeHtml(msg.role || 'unknown');
        var roleClass = role.replace(/[^a-zA-Z0-9-]/g, '');
        return '<div class="chat-message ' + roleClass + '">' +
            '<div class="chat-message-role">' + role + '</div>' +
            '<div class="chat-message-content">' + escapeHtml(msg.content) + '</div>' +
            '</div>';
    }).join('');
    container.scrollTop = container.scrollHeight;
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
