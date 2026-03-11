let ws = null;
let currentPlan = null;
let undoStack = [];
let redoStack = [];
let isGenerating = false;
let componentAccumulator = {};

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
    get mcpStatusContent() { return ensureElement('mcp-status-content'); },
    get remoteLlmStatusContent() { return ensureElement('remote-llm-status-content'); },
    get statusUpdateTime() { return ensureElement('status-update-time'); },
    get notificationContainer() { return ensureElement('notification-container'); }
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
                <h4>MCP Tools</h4>
                <div id="mcp-status-content">
                    <div class="status-item">
                        <span class="status-label">Loading...</span>
                        <span class="status-value">...</span>
                    </div>
                </div>
            </div>

            <div class="status-section">
                <h4>LLMs</h4>
                <div id="remote-llm-status-content">
                    <div class="status-item">
                        <span class="status-label">Loading...</span>
                        <span class="status-value">...</span>
                    </div>
                </div>
                <div class="status-update-time" id="status-update-time">Never</div>
            </div>
        `;
        mainContainer.insertBefore(statusPanel, mainContainer.firstChild);
        console.log('Status panel repaired');
    }

    // Ensure notification container exists
    let notificationContainer = document.getElementById('notification-container');
    if (!notificationContainer) {
        notificationContainer = document.createElement('div');
        notificationContainer.id = 'notification-container';
        document.body.appendChild(notificationContainer);
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
        statusUpdate: handleStatusUpdate,
        systemNotification: handleSystemNotification
    };

    if (Object.prototype.hasOwnProperty.call(handlers, message.type)) {
        const handler = handlers[message.type];
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

        const nameDiv = document.createElement('div');
        nameDiv.className = 'plan-item-name';
        nameDiv.textContent = part.name;

        const descDiv = document.createElement('div');
        descDiv.className = 'plan-item-desc';
        descDiv.textContent = part.description;

        const statusDiv = document.createElement('div');
        statusDiv.className = 'plan-item-status pending';
        statusDiv.textContent = 'Pending';

        item.appendChild(nameDiv);
        item.appendChild(descDiv);
        item.appendChild(statusDiv);
        elements.planItems.appendChild(item);
    });

    if (elements.hintOverlay) {
        elements.hintOverlay.remove();
    }

    // Automatically start generating the first component
    if (data.parts.length > 0) {
        const firstPart = data.parts[0];
        console.log('Starting generation for first part:', firstPart.id);
        sendMessage({ type: 'applyPart', partId: firstPart.id });
    }
}

function handlePartStream(data) {
    const planItem = document.querySelector(`[data-part-id="${data.partId}"]`);
    if (planItem) {
        const status = planItem.querySelector('.plan-item-status');

        // Initialize accumulator for this part if needed
        if (!componentAccumulator[data.partId]) {
            componentAccumulator[data.partId] = {
                html: '',
                css: '',
                js: ''
            };
        }

        // Accumulate content by type
        if (!data.done && data.content) {
            if (data.chunkType === 'html') {
                componentAccumulator[data.partId].html += data.content;
            } else if (data.chunkType === 'css') {
                componentAccumulator[data.partId].css += data.content;
            } else if (data.chunkType === 'js') {
                componentAccumulator[data.partId].js += data.content;
            }

            status.className = 'plan-item-status generating';
            status.textContent = `Generating ${data.chunkType}...`;
        }

        if (data.done) {
            status.className = 'plan-item-status';
            status.textContent = 'Complete';

            // Inject the accumulated code into the DOM
            const accumulated = componentAccumulator[data.partId];
            if (accumulated) {
                injectComponent(data.partId, accumulated);
            }

            // Find the next component to generate
            const currentIndex = currentPlan.findIndex(p => p.id === data.partId);
            if (currentIndex >= 0 && currentIndex < currentPlan.length - 1) {
                const nextPart = currentPlan[currentIndex + 1];
                console.log('Starting generation for next part:', nextPart.id);
                sendMessage({ type: 'applyPart', partId: nextPart.id });
            } else {
                console.log('All components generated');
                updateStatus('connected', 'Generation complete');
                isGenerating = false;
                elements.generateBtn.disabled = false;
                elements.cancelBtn.disabled = true;
            }
        }
    }
}

function injectComponent(partId, code) {
    const stage = elements.stage;

    // Remove hint overlay if it exists
    if (elements.hintOverlay) {
        elements.hintOverlay.remove();
    }

    // Create a container for this component
    const container = document.createElement('div');
    container.id = `component-${partId}`;
    container.className = 'component-container';

    // Inject HTML
    if (code.html) {
        container.innerHTML = code.html;
    }

    // Inject CSS
    if (code.css) {
        const style = document.createElement('style');
        style.id = `style-${partId}`;
        style.textContent = code.css;
        document.head.appendChild(style);
    }

    // Append to stage
    stage.appendChild(container);

    // Inject JavaScript (after HTML is in DOM)
    if (code.js) {
        const script = document.createElement('script');
        script.id = `script-${partId}`;
        script.textContent = code.js;
        document.body.appendChild(script);
    }

    console.log(`Injected component ${partId} into DOM`);
}

function handleAppliedPart(data) {
    undoStack.push(data.version_id);
    redoStack = [];
    updateUndoRedoButtons();

    // Find the part name from the current plan
    const part = currentPlan && currentPlan.find(p => p.id === data.part_id);
    const partName = part ? part.name : data.part_id;
    updateStatus('connected', `Applied ${partName}`);
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

function handleSystemNotification(data) {
    showNotification(data.message, data.severity || 'info');
}

function showNotification(message, severity) {
    const container = elements.notificationContainer;
    if (!container) return;

    const notification = document.createElement('div');
    notification.className = `notification notification-${severity}`;
    notification.textContent = message;

    container.appendChild(notification);

    // Auto-dismiss after 8 seconds for info, 15 seconds for warning/error
    const duration = severity === 'info' ? 8000 : 15000;
    setTimeout(() => {
        notification.classList.add('notification-fade-out');
        setTimeout(() => notification.remove(), 300);
    }, duration);
}

function updateUndoRedoButtons() {
    elements.undoBtn.disabled = undoStack.length === 0;
    elements.redoBtn.disabled = redoStack.length === 0;
}

elements.generateBtn.addEventListener('click', () => {
    const prompt = elements.promptInput.value.trim();
    if (!prompt) return;

    sendMessage({
        type: 'submitPrompt',
        text: prompt
    });

    isGenerating = true;
    elements.generateBtn.disabled = true;
    elements.cancelBtn.disabled = false;
    updateStatus('connected', 'Planning...');
});

elements.cancelBtn.addEventListener('click', () => {
    sendMessage({ type: 'cancelGeneration' });
    isGenerating = false;
    elements.generateBtn.disabled = false;
    elements.cancelBtn.disabled = true;
    updateStatus('connected', 'Cancelled');
});

elements.undoBtn.addEventListener('click', () => {
    if (undoStack.length > 0) {
        const versionId = undoStack.pop();
        redoStack.push(versionId);
        sendMessage({ type: 'undo' });
        updateUndoRedoButtons();
    }
});

elements.redoBtn.addEventListener('click', () => {
    if (redoStack.length > 0) {
        const versionId = redoStack.pop();
        undoStack.push(versionId);
        sendMessage({ type: 'redo' });
        updateUndoRedoButtons();
    }
});

elements.promptInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        elements.generateBtn.click();
    }
});

function escapeHtmlToolbar(text) {
    var d = document.createElement('div');
    d.textContent = text;
    return d.innerHTML;
}

function handleStatusUpdate(data) {
    // Health monitoring happens silently in background
    // Only user-facing notifications will be shown via SystemNotification events

    if (elements.mcpStatusContent) {
        var mcpHtml =
            '<div class="status-item">' +
                '<span class="status-label">Browser CDP</span>' +
                '<span class="status-value ' + (data.mcpTools.browser_available ? 'good' : 'error') + '">' +
                    (data.mcpTools.browser_available ? 'Available' : 'Unavailable') +
                '</span>' +
            '</div>' +
            '<div class="status-item">' +
                '<span class="status-label">Tools</span>' +
                '<span class="status-value">' + escapeHtmlToolbar(String(data.mcpTools.tools_count)) + '</span>' +
            '</div>';
        if (data.mcpTools.last_used) {
            mcpHtml +=
                '<div class="status-item">' +
                    '<span class="status-label">Last Used</span>' +
                    '<span class="status-value">' + escapeHtmlToolbar(String(data.mcpTools.last_used)) + '</span>' +
                '</div>';
        }
        elements.mcpStatusContent.innerHTML = mcpHtml;
    }

    // Display available LLMs
    if (elements.remoteLlmStatusContent && data.llms) {
        var llmHtml = data.llms.map(function(llm) {
            return '<div class="llm-entry">' +
                '<div class="llm-name">' + escapeHtmlToolbar(llm.name) + '</div>' +
                '<div class="status-item">' +
                    '<span class="status-label">Provider</span>' +
                    '<span class="status-value">' + escapeHtmlToolbar(llm.provider) + '</span>' +
                '</div>' +
                '<div class="status-item">' +
                    '<span class="status-label">Model</span>' +
                    '<span class="status-value">' + escapeHtmlToolbar(llm.model) + '</span>' +
                '</div>' +
                '<div class="status-item">' +
                    '<span class="status-label">Status</span>' +
                    '<span class="status-value ' + (llm.connected ? 'good' : 'error') + '">' +
                        (llm.connected ? 'Ready' : 'Unavailable') +
                    '</span>' +
                '</div>' +
            '</div>';
        }).join('');
        elements.remoteLlmStatusContent.innerHTML = llmHtml;
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
            .llm-entry {
                background: #2a2a2a;
                border: 1px solid #3a3a3a;
                border-radius: 3px;
                padding: 8px;
                margin-bottom: 8px;
            }
            .llm-entry:last-child {
                margin-bottom: 0;
            }
            .llm-name {
                font-size: 13px;
                font-weight: 600;
                color: #e0e0e0;
                margin-bottom: 6px;
                border-bottom: 1px solid #3a3a3a;
                padding-bottom: 4px;
            }

            #notification-container {
                position: fixed;
                bottom: 24px;
                right: 24px;
                z-index: 1000;
                display: flex;
                flex-direction: column;
                gap: 12px;
                max-width: 400px;
            }
            .notification {
                background: #1a1a1a;
                border: 1px solid #3a3a3a;
                border-radius: 6px;
                padding: 14px 16px;
                font-size: 14px;
                line-height: 1.5;
                color: #e0e0e0;
                box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
                animation: notification-slide-in 0.3s ease-out;
            }
            .notification-info {
                border-left: 3px solid #3b82f6;
            }
            .notification-warning {
                border-left: 3px solid #fbbf24;
            }
            .notification-error {
                border-left: 3px solid #ef4444;
            }
            .notification-fade-out {
                animation: notification-fade-out 0.3s ease-out forwards;
            }
            @keyframes notification-slide-in {
                from {
                    transform: translateX(400px);
                    opacity: 0;
                }
                to {
                    transform: translateX(0);
                    opacity: 1;
                }
            }
            @keyframes notification-fade-out {
                from {
                    transform: translateX(0);
                    opacity: 1;
                }
                to {
                    transform: translateX(400px);
                    opacity: 0;
                }
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
