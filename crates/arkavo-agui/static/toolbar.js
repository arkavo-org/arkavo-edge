let ws = null;
let currentPlan = null;
let undoStack = [];
let redoStack = [];
let isGenerating = false;

const elements = {
    promptInput: document.getElementById('prompt-input'),
    generateBtn: document.getElementById('generate-btn'),
    cancelBtn: document.getElementById('cancel-btn'),
    undoBtn: document.getElementById('undo-btn'),
    redoBtn: document.getElementById('redo-btn'),
    status: document.getElementById('status'),
    stage: document.getElementById('stage'),
    planPanel: document.getElementById('plan-panel'),
    planItems: document.getElementById('plan-items'),
    hintOverlay: document.getElementById('hint-overlay'),
    sandbox: document.getElementById('sandbox'),
    get systemStatusContent() { return ensureElement('system-status-content'); },
    get mcpStatusContent() { return ensureElement('mcp-status-content'); },
    get remoteLlmStatusContent() { return ensureElement('remote-llm-status-content'); },
    get statusUpdateTime() { return ensureElement('status-update-time'); }
};

function ensureElement(id) {
    let el = document.getElementById(id);
    if (!el) {
        console.log(`Creating missing element: ${id}`);
        el = repairStatusPanel();
        el = document.getElementById(id);
    }
    return el;
}

function repairStatusPanel() {
    let statusPanel = document.getElementById('status-panel');
    if (!statusPanel) {
        const mainContainer = document.getElementById('main-container');
        if (!mainContainer) return null;

        statusPanel = document.createElement('aside');
        statusPanel.id = 'status-panel';
        statusPanel.innerHTML = `
            <div class="status-section">
                <h4>System Status</h4>
                <div id="system-status-content">
                    <div class="status-item">
                        <span class="status-label">Loading...</span>
                        <span class="status-value">...</span>
                    </div>
                </div>
                <div class="status-update-time" id="status-update-time">Never</div>
            </div>

            <div class="status-section">
                <h4>MCP Tools</h4>
                <div id="mcp-status-content">
                    <div class="status-item">
                        <span class="status-label">Loading...</span>
                        <span class="status-value">...</span>
                    </div>
                </div>
            </div>

            <div class="status-section">
                <h4>Remote LLM</h4>
                <div id="remote-llm-status-content">
                    <div class="status-item">
                        <span class="status-label">Loading...</span>
                        <span class="status-value">...</span>
                    </div>
                </div>
            </div>
        `;
        mainContainer.insertBefore(statusPanel, mainContainer.firstChild);
        console.log('Status panel repaired');
    }
    return statusPanel;
}

function connectWebSocket() {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/ws`;

    ws = new WebSocket(wsUrl);

    ws.onopen = () => {
        updateStatus('connected', 'Connected');
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
        undoAvailable: handleUndoAvailable,
        statusUpdate: handleStatusUpdate
    };

    const handler = handlers[message.type];
    if (handler) {
        // For Plan event, data is directly in the message (not nested under data)
        if (message.type === 'plan') {
            handler(message);
        } else {
            handler(message.data || message);
        }
    } else {
        console.warn('No handler for message type:', message.type);
    }
}

function handlePlan(data) {
    currentPlan = data.parts;
    elements.planPanel.classList.add('visible');
    elements.planItems.innerHTML = '';

    data.parts.forEach(part => {
        const item = document.createElement('div');
        item.className = 'plan-item';
        item.dataset.partId = part.id;
        item.innerHTML = `
            <div class="plan-item-name">${part.name}</div>
            <div class="plan-item-desc">${part.description}</div>
            <div class="plan-item-status pending">Pending</div>
        `;
        elements.planItems.appendChild(item);
    });

    if (elements.hintOverlay) {
        elements.hintOverlay.remove();
    }
}

function handlePartStream(data) {
    const planItem = document.querySelector(`[data-part-id="${data.part_id}"]`);
    if (planItem) {
        const status = planItem.querySelector('.plan-item-status');
        if (data.done) {
            status.className = 'plan-item-status';
            status.textContent = 'Complete';
            sendMessage({ type: 'ApplyPart', data: { part_id: data.part_id } });
        } else {
            status.className = 'plan-item-status generating';
            status.textContent = `Generating ${data.chunk_type}...`;
        }
    }
}

function handleAppliedPart(data) {
    undoStack.push(data.version_id);
    redoStack = [];
    updateUndoRedoButtons();
    updateStatus('connected', `Applied ${data.part_id}`);
}

function handleError(data) {
    updateStatus('error', data.message);
    isGenerating = false;
    elements.generateBtn.disabled = false;
    elements.cancelBtn.disabled = true;
}

function handleUndoAvailable(data) {
    elements.undoBtn.disabled = !data.can_undo;
    elements.redoBtn.disabled = !data.can_redo;
}

function updateUndoRedoButtons() {
    elements.undoBtn.disabled = undoStack.length === 0;
    elements.redoBtn.disabled = redoStack.length === 0;
}

elements.generateBtn.addEventListener('click', () => {
    const prompt = elements.promptInput.value.trim();
    if (!prompt) return;

    sendMessage({
        type: 'SubmitPrompt',
        data: { text: prompt }
    });

    isGenerating = true;
    elements.generateBtn.disabled = true;
    elements.cancelBtn.disabled = false;
    updateStatus('connected', 'Planning...');
});

elements.cancelBtn.addEventListener('click', () => {
    sendMessage({ type: 'CancelGeneration' });
    isGenerating = false;
    elements.generateBtn.disabled = false;
    elements.cancelBtn.disabled = true;
    updateStatus('connected', 'Cancelled');
});

elements.undoBtn.addEventListener('click', () => {
    if (undoStack.length > 0) {
        const versionId = undoStack.pop();
        redoStack.push(versionId);
        sendMessage({ type: 'Undo' });
        updateUndoRedoButtons();
    }
});

elements.redoBtn.addEventListener('click', () => {
    if (redoStack.length > 0) {
        const versionId = redoStack.pop();
        undoStack.push(versionId);
        sendMessage({ type: 'Redo' });
        updateUndoRedoButtons();
    }
});

elements.promptInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        elements.generateBtn.click();
    }
});

function handleStatusUpdate(data) {
    if (elements.systemStatusContent) {
        elements.systemStatusContent.innerHTML = `
            <div class="status-item">
                <span class="status-label">Uptime</span>
                <span class="status-value">${data.system.uptime}</span>
            </div>
            <div class="status-item">
                <span class="status-label">Memory</span>
                <span class="status-value">${data.system.memory_usage}</span>
            </div>
            <div class="status-item">
                <span class="status-label">Connections</span>
                <span class="status-value good">${data.system.active_connections}</span>
            </div>
        `;
    }

    if (elements.mcpStatusContent) {
        elements.mcpStatusContent.innerHTML = `
            <div class="status-item">
                <span class="status-label">Browser CDP</span>
                <span class="status-value ${data.mcpTools.browser_available ? 'good' : 'error'}">
                    ${data.mcpTools.browser_available ? 'Available' : 'Unavailable'}
                </span>
            </div>
            <div class="status-item">
                <span class="status-label">Tools</span>
                <span class="status-value">${data.mcpTools.tools_count}</span>
            </div>
            ${data.mcpTools.last_used ? `
            <div class="status-item">
                <span class="status-label">Last Used</span>
                <span class="status-value">${data.mcpTools.last_used}</span>
            </div>
            ` : ''}
        `;
    }

    if (elements.remoteLlmStatusContent) {
        elements.remoteLlmStatusContent.innerHTML = `
            <div class="status-item">
                <span class="status-label">Status</span>
                <span class="status-value ${data.remoteLlm.connected ? 'good' : 'error'}">
                    ${data.remoteLlm.connected ? 'Connected' : 'Disconnected'}
                </span>
            </div>
            <div class="status-item">
                <span class="status-label">Model</span>
                <span class="status-value">${data.remoteLlm.model}</span>
            </div>
            <div class="status-item">
                <span class="status-label">Requests</span>
                <span class="status-value">${data.remoteLlm.requests_today}</span>
            </div>
        `;
    }

    if (elements.statusUpdateTime) {
        const timestamp = new Date(data.timestamp);
        elements.statusUpdateTime.textContent = `Updated: ${timestamp.toLocaleTimeString()}`;
    }
}

function requestStatusUpdate() {
    sendMessage({ type: 'requestStatus' });
}

function ensureStatusPanelStyles() {
    if (!document.getElementById('status-panel-styles')) {
        const style = document.createElement('style');
        style.id = 'status-panel-styles';
        style.textContent = `
            #status-panel {
                width: 280px;
                background: #2a2a2a;
                border-right: 1px solid #3a3a3a;
                padding: 16px;
                overflow-y: auto;
                display: flex;
                flex-direction: column;
                gap: 12px;
            }
            .status-section {
                background: #1a1a1a;
                border: 1px solid #3a3a3a;
                border-radius: 4px;
                padding: 12px;
            }
            .status-section h4 {
                font-size: 12px;
                font-weight: 600;
                color: #888;
                margin-bottom: 8px;
                text-transform: uppercase;
                letter-spacing: 0.5px;
            }
            .status-item {
                display: flex;
                justify-content: space-between;
                align-items: center;
                padding: 6px 0;
                border-bottom: 1px solid #2a2a2a;
                font-size: 13px;
            }
            .status-item:last-child {
                border-bottom: none;
            }
            .status-label {
                color: #aaa;
            }
            .status-value {
                color: #e0e0e0;
                font-weight: 500;
            }
            .status-value.good {
                color: #4ade80;
            }
            .status-value.warning {
                color: #fbbf24;
            }
            .status-value.error {
                color: #ef4444;
            }
            .status-update-time {
                font-size: 10px;
                color: #666;
                text-align: right;
                margin-top: 8px;
            }
        `;
        document.head.appendChild(style);
        console.log('Status panel styles ensured');
    }
}

ensureStatusPanelStyles();
repairStatusPanel();

connectWebSocket();

setInterval(requestStatusUpdate, 30000);
setInterval(() => {
    ensureStatusPanelStyles();
    repairStatusPanel();
}, 5000);

requestStatusUpdate();
