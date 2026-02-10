"use strict";

var currentChat = null;
var chatMessages = [];

function openAgentChat(agentId) {
    currentChat = agentId;
    chatMessages = [];
    document.getElementById('chat-agent-name').textContent = agentId;
    document.getElementById('chat-messages').innerHTML = '';
    document.getElementById('chat-panel').classList.add('active');
    document.getElementById('chat-input').focus();

    wsSend({ type: 'chatOpen', agentId: agentId });
}

function closeChat() {
    if (currentChat) {
        wsSend({ type: 'chatClose', agentId: currentChat });
    }
    currentChat = null;
    chatMessages = [];
    document.getElementById('chat-panel').classList.remove('active');
}

function handleMessagesSnapshot(event) {
    if (event.messages) {
        chatMessages = event.messages;
        renderChatMessages();
    }
}

function handleMessageDelta(event) {
    if (event.agentId !== currentChat && event.agent_id !== currentChat) return;

    var delta = event.delta;
    if (delta && delta.type === 'text') {
        var msgId = event.messageId || event.message_id;
        var existingMsg = chatMessages.find(function(m) { return m.id === msgId; });
        if (!existingMsg) {
            existingMsg = { id: msgId, role: 'assistant', content: '' };
            chatMessages.push(existingMsg);
        }
        existingMsg.content += delta.text || '';
        renderChatMessages();
    }
}

function renderChatMessages() {
    var container = document.getElementById('chat-messages');
    container.innerHTML = chatMessages.map(function(msg) {
        var role = escapeHtml(msg.role || 'unknown');
        var roleClass = role.replace(/[^a-zA-Z0-9-]/g, '');
        return '<div class="chat-message ' + roleClass + '">' +
            '<div class="chat-message-role">' + role + '</div>' +
            '<div class="chat-message-content">' + escapeHtml(msg.content) + '</div>' +
            '</div>';
    }).join('');
    container.scrollTop = container.scrollHeight;
}

function sendChatMessage() {
    var input = document.getElementById('chat-input');
    var content = input.value.trim();

    if (!content || !currentChat) return;

    chatMessages.push({ id: Date.now().toString(), role: 'user', content: content });
    renderChatMessages();

    wsSend({ type: 'userMessage', agentId: currentChat, content: content });
    input.value = '';
}

function handleChatKeypress(event) {
    if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        sendChatMessage();
    }
}
