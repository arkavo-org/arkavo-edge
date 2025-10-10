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
    sandbox: document.getElementById('sandbox')
};

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
    const handlers = {
        Plan: handlePlan,
        PartStream: handlePartStream,
        AppliedPart: handleAppliedPart,
        Error: handleError,
        UndoAvailable: handleUndoAvailable
    };

    const handler = handlers[message.type];
    if (handler) {
        handler(message.data);
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

connectWebSocket();
