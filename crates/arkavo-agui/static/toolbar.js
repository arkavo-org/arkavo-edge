let ws = null;
let sse = null;
let currentPlan = null;
let undoStack = [];
let redoStack = [];
let isGenerating = false;
let componentAccumulator = {};
let discoveredAgents = [];
let selectedAgentId = null;
let chatMessages = {};

const elements = {
    promptInput: document.getElementById('prompt-input'),
    generateBtn: document.getElementById('generate-btn'),
    cancelBtn: document.getElementById('cancel-btn'),
    status: document.getElementById('status'),
    stage: document.getElementById('stage'),
    hintOverlay: document.getElementById('hint-overlay'),
    agentsList: document.getElementById('agents-list'),
    chatPanel: document.getElementById('chat-panel'),
    chatAgentName: document.getElementById('chat-agent-name'),
    chatMessages: document.getElementById('chat-messages'),
    chatInput: document.getElementById('chat-input'),
    sendChatBtn: document.getElementById('send-chat-btn'),
    closeChatBtn: document.getElementById('close-chat-btn'),
    get mcpStatusContent() { return document.getElementById('mcp-status-content'); },
    get remoteLlmStatusContent() { return document.getElementById('remote-llm-status-content'); },
    get statusUpdateTime() { return document.getElementById('status-update-time'); }
};

// Connect to WebSocket for AG-UI protocol
function connectWebSocket() {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/ws`;

    ws = new WebSocket(wsUrl);

    ws.onopen = () => {
        updateStatus('connected', 'Connected');
        requestStatusUpdate();
    };

    ws.onmessage = (event) => {
        const message = JSON.parse(event.data);
        handleMessage(message);
    };

    ws.onerror = () => {
        updateStatus('error', 'Connection error');
    };

    ws.onclose = () => {
        updateStatus('error', 'Disconnected');
        setTimeout(connectWebSocket, 3000);
    };
}

// Connect to SSE for agent discovery
function connectSSE() {
    sse = new EventSource('/events');

    sse.addEventListener('discovery', (event) => {
        const data = JSON.parse(event.data);
        if (data.agents) {
            discoveredAgents = data.agents;
            renderAgentsList();
        }
    });

    sse.onerror = () => {
        console.log('SSE connection error, reconnecting...');
        sse.close();
        setTimeout(connectSSE, 5000);
    };
}

function updateStatus(className, text) {
    elements.status.className = className;
    elements.status.textContent = text;
}

function sendMessage(message) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify(message));
    }
}

function handleMessage(message) {
    console.log('Received message:', message);

    const handlers = {
        plan: handlePlan,
        partStream: handlePartStream,
        appliedPart: handleAppliedPart,
        error: handleError,
        statusUpdate: handleStatusUpdate,
        systemNotification: handleSystemNotification,
        // AG-UI chat events
        assistantChunk: handleAssistantChunk,
        assistantMessage: handleAssistantMessage,
        toolCallStart: handleToolCallStart,
        toolCallEnd: handleToolCallEnd,
        // Message delta from agent chat
        messageDelta: handleMessageDelta
    };

    const handler = handlers[message.type];
    if (handler) {
        if (message.type === 'plan') {
            handler(message);
        } else {
            handler(message.data || message);
        }
    }
}

// Agent list rendering
function renderAgentsList() {
    if (!elements.agentsList) return;

    if (discoveredAgents.length === 0) {
        elements.agentsList.innerHTML = `
            <div class="status-item">
                <span class="status-label">No agents found</span>
            </div>
        `;
        return;
    }

    elements.agentsList.innerHTML = discoveredAgents.map(agent => {
        const agentId = agent.id || agent.agent_id || 'unknown';
        const name = agent.metadata?.name || agent.name || agentId;
        const model = agent.metadata?.model || 'unknown';
        const mcpTools = agent.metadata?.mcp_tools || [];
        const isSelected = selectedAgentId === agentId;

        return `
            <div class="agent-card ${isSelected ? 'selected' : ''}" data-agent-id="${agentId}">
                <div class="agent-card-header">
                    <span class="agent-name">${escapeHtml(name)}</span>
                    <span class="agent-status">Online</span>
                </div>
                <div class="agent-details">
                    <div>Model: ${escapeHtml(model)}</div>
                    <div>Endpoint: ${escapeHtml(agent.endpoint || 'N/A')}</div>
                </div>
                ${mcpTools.length > 0 ? `
                    <div class="agent-tools">
                        ${mcpTools.slice(0, 5).map(tool => `
                            <span class="agent-tool-tag">${escapeHtml(tool)}</span>
                        `).join('')}
                        ${mcpTools.length > 5 ? `<span class="agent-tool-tag">+${mcpTools.length - 5} more</span>` : ''}
                    </div>
                ` : ''}
            </div>
        `;
    }).join('');

    // Add click handlers
    elements.agentsList.querySelectorAll('.agent-card').forEach(card => {
        card.addEventListener('click', () => {
            const agentId = card.dataset.agentId;
            selectAgent(agentId);
        });
    });
}

function selectAgent(agentId) {
    // Close previous chat if open
    if (selectedAgentId && selectedAgentId !== agentId) {
        sendMessage({ type: 'chatClose', agentId: selectedAgentId });
    }

    selectedAgentId = agentId;
    renderAgentsList();

    // Find agent info
    const agent = discoveredAgents.find(a => (a.id || a.agent_id) === agentId);
    const name = agent?.metadata?.name || agent?.name || agentId;

    // Open chat panel
    elements.chatAgentName.textContent = name;
    elements.chatPanel.classList.add('visible');

    // Initialize chat messages for this agent if needed
    if (!chatMessages[agentId]) {
        chatMessages[agentId] = [];
    }
    renderChatMessages();

    // Send chatOpen event
    sendMessage({ type: 'chatOpen', agentId: agentId });

    // Add system message
    addChatMessage(agentId, 'system', `Connected to ${name}`);

    // Hide hint overlay
    if (elements.hintOverlay) {
        elements.hintOverlay.style.display = 'none';
    }
}

function closeChat() {
    if (selectedAgentId) {
        sendMessage({ type: 'chatClose', agentId: selectedAgentId });
    }
    elements.chatPanel.classList.remove('visible');
    selectedAgentId = null;
    renderAgentsList();
}

function sendChatMessage() {
    const content = elements.chatInput.value.trim();
    if (!content || !selectedAgentId) return;

    // Add user message to UI
    addChatMessage(selectedAgentId, 'user', content);

    // Send to agent via AG-UI protocol
    sendMessage({
        type: 'userMessage',
        agentId: selectedAgentId,
        content: content,
        attachments: []
    });

    // Clear input
    elements.chatInput.value = '';
    elements.chatInput.style.height = 'auto';
}

function addChatMessage(agentId, role, content) {
    if (!chatMessages[agentId]) {
        chatMessages[agentId] = [];
    }
    chatMessages[agentId].push({ role, content, timestamp: new Date() });
    renderChatMessages();
}

function renderChatMessages() {
    if (!selectedAgentId || !elements.chatMessages) return;

    const messages = chatMessages[selectedAgentId] || [];
    elements.chatMessages.innerHTML = messages.map(msg => `
        <div class="chat-message ${msg.role}">
            ${escapeHtml(msg.content)}
        </div>
    `).join('');

    // Scroll to bottom
    elements.chatMessages.scrollTop = elements.chatMessages.scrollHeight;
}

// AG-UI chat event handlers
function handleAssistantChunk(data) {
    // Streaming chunk from agent
    if (data.agentId && data.content) {
        // For streaming, append to last agent message or create new
        const messages = chatMessages[data.agentId] || [];
        const lastMsg = messages[messages.length - 1];

        if (lastMsg && lastMsg.role === 'agent' && lastMsg.streaming) {
            lastMsg.content += data.content;
        } else {
            chatMessages[data.agentId] = chatMessages[data.agentId] || [];
            chatMessages[data.agentId].push({
                role: 'agent',
                content: data.content,
                streaming: true,
                timestamp: new Date()
            });
        }
        renderChatMessages();
    }
}

function handleAssistantMessage(data) {
    // Complete message from agent
    if (data.agentId && data.content) {
        // Mark last message as complete
        const messages = chatMessages[data.agentId] || [];
        const lastMsg = messages[messages.length - 1];
        if (lastMsg && lastMsg.streaming) {
            lastMsg.streaming = false;
            lastMsg.content = data.content;
        } else {
            addChatMessage(data.agentId, 'agent', data.content);
        }
        renderChatMessages();
    }
}

function handleToolCallStart(data) {
    if (data.agentId && data.toolName) {
        addChatMessage(data.agentId, 'system', `Calling tool: ${data.toolName}...`);
    }
}

function handleToolCallEnd(data) {
    if (data.agentId && data.toolName) {
        const status = data.success ? '✓' : '✗';
        addChatMessage(data.agentId, 'system', `Tool ${data.toolName} ${status}`);
    }
}

// Handle message delta from agent chat sessions
function handleMessageDelta(data) {
    const agentId = data.agentId;
    if (!agentId) return;

    // Handle different delta types
    if (data.delta) {
        if (data.delta.text !== undefined) {
            // Text content
            const messages = chatMessages[agentId] || [];
            const lastMsg = messages[messages.length - 1];

            if (lastMsg && lastMsg.role === 'agent' && lastMsg.streaming) {
                // Append to existing streaming message
                lastMsg.content += data.delta.text;
            } else {
                // Start new streaming message
                chatMessages[agentId] = chatMessages[agentId] || [];
                chatMessages[agentId].push({
                    role: 'agent',
                    content: data.delta.text,
                    streaming: true,
                    timestamp: new Date()
                });
            }
            renderChatMessages();
        } else if (data.delta.toolCallId !== undefined) {
            // Tool call delta
            if (data.delta.name) {
                addChatMessage(agentId, 'system', `Calling tool: ${data.delta.name}...`);
            }
        } else if (data.delta.toolResult !== undefined) {
            // Tool result
            addChatMessage(agentId, 'system', `Tool result received`);
        }
    }
}

// UI generation handlers (existing)
function handlePlan(data) {
    currentPlan = data.parts;

    // Clear stage and show plan
    elements.stage.innerHTML = '';
    if (elements.hintOverlay) {
        elements.hintOverlay.style.display = 'none';
    }

    // Auto-generate first part
    if (data.parts.length > 0) {
        sendMessage({ type: 'applyPart', partId: data.parts[0].id });
    }
}

function handlePartStream(data) {
    if (!componentAccumulator[data.partId]) {
        componentAccumulator[data.partId] = { html: '', css: '', js: '' };
    }

    if (!data.done && data.content) {
        componentAccumulator[data.partId][data.chunkType] += data.content;
    }

    if (data.done) {
        injectComponent(data.partId, componentAccumulator[data.partId]);

        // Generate next part
        const currentIndex = currentPlan.findIndex(p => p.id === data.partId);
        if (currentIndex >= 0 && currentIndex < currentPlan.length - 1) {
            sendMessage({ type: 'applyPart', partId: currentPlan[currentIndex + 1].id });
        } else {
            isGenerating = false;
            elements.generateBtn.disabled = false;
            elements.cancelBtn.disabled = true;
        }
    }
}

function injectComponent(partId, code) {
    const container = document.createElement('div');
    container.id = `component-${partId}`;
    container.className = 'component-container';

    if (code.html) container.innerHTML = code.html;

    if (code.css) {
        const style = document.createElement('style');
        style.textContent = code.css;
        document.head.appendChild(style);
    }

    elements.stage.appendChild(container);

    if (code.js) {
        const script = document.createElement('script');
        script.textContent = code.js;
        document.body.appendChild(script);
    }
}

function handleAppliedPart(data) {
    undoStack.push(data.version_id);
    redoStack = [];
}

function handleError(data) {
    updateStatus('error', data.message);
    isGenerating = false;
    elements.generateBtn.disabled = false;
    elements.cancelBtn.disabled = true;
}

function handleStatusUpdate(data) {
    if (elements.mcpStatusContent) {
        elements.mcpStatusContent.innerHTML = `
            <div class="status-item">
                <span class="status-label">Browser CDP</span>
                <span class="status-value ${data.mcpTools?.browser_available ? 'good' : 'error'}">
                    ${data.mcpTools?.browser_available ? 'Available' : 'Unavailable'}
                </span>
            </div>
            <div class="status-item">
                <span class="status-label">Tools</span>
                <span class="status-value">${data.mcpTools?.tools_count || 0}</span>
            </div>
        `;
    }

    if (elements.remoteLlmStatusContent && data.llms) {
        elements.remoteLlmStatusContent.innerHTML = data.llms.map(llm => `
            <div class="llm-entry">
                <div class="llm-name">${llm.name}</div>
                <div class="status-item">
                    <span class="status-label">Status</span>
                    <span class="status-value ${llm.connected ? 'good' : 'error'}">
                        ${llm.connected ? 'Ready' : 'Unavailable'}
                    </span>
                </div>
            </div>
        `).join('');
    }

    if (elements.statusUpdateTime && data.timestamp) {
        const timestamp = new Date(data.timestamp);
        elements.statusUpdateTime.textContent = `Updated: ${timestamp.toLocaleTimeString()}`;
    }
}

function handleSystemNotification(data) {
    console.log('System notification:', data.message);
}

function requestStatusUpdate() {
    sendMessage({ type: 'requestStatus' });
}

function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// Event listeners
elements.generateBtn.addEventListener('click', () => {
    const prompt = elements.promptInput.value.trim();
    if (!prompt) return;

    // If an agent is selected, send as chat message
    if (selectedAgentId) {
        addChatMessage(selectedAgentId, 'user', prompt);
        sendMessage({
            type: 'userMessage',
            agentId: selectedAgentId,
            content: prompt,
            attachments: []
        });
        elements.promptInput.value = '';
    } else {
        // Otherwise, generate UI
        sendMessage({ type: 'submitPrompt', text: prompt });
        isGenerating = true;
        elements.generateBtn.disabled = true;
        elements.cancelBtn.disabled = false;
        updateStatus('connected', 'Planning...');
    }
});

elements.cancelBtn.addEventListener('click', () => {
    sendMessage({ type: 'cancelGeneration' });
    isGenerating = false;
    elements.generateBtn.disabled = false;
    elements.cancelBtn.disabled = true;
    updateStatus('connected', 'Cancelled');
});

elements.promptInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        elements.generateBtn.click();
    }
});

if (elements.closeChatBtn) {
    elements.closeChatBtn.addEventListener('click', closeChat);
}

if (elements.sendChatBtn) {
    elements.sendChatBtn.addEventListener('click', sendChatMessage);
}

if (elements.chatInput) {
    elements.chatInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            sendChatMessage();
        }
    });

    // Auto-resize textarea
    elements.chatInput.addEventListener('input', () => {
        elements.chatInput.style.height = 'auto';
        elements.chatInput.style.height = Math.min(elements.chatInput.scrollHeight, 120) + 'px';
    });
}

// Initialize
connectWebSocket();
connectSSE();
setInterval(requestStatusUpdate, 30000);
