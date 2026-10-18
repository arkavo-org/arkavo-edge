"use strict";

var taskSubmission = null;
var taskReceiptTimer = null;

// crypto.randomUUID exists only in secure contexts (HTTPS or localhost), and
// the workspace is also opened over a LAN address.
function newRequestId() {
    if (typeof crypto.randomUUID === 'function') return crypto.randomUUID();
    var bytes = crypto.getRandomValues(new Uint8Array(16));
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    var hex = Array.from(bytes, function(b) { return (b + 0x100).toString(16).slice(1); }).join('');
    return hex.slice(0, 8) + '-' + hex.slice(8, 12) + '-' + hex.slice(12, 16) + '-' + hex.slice(16, 20) + '-' + hex.slice(20);
}

function showAddTaskModal() {
    var dialog = document.getElementById('add-task-modal');
    updateComposerAvailability();
    if (!dialog.open) dialog.showModal();
    document.getElementById('task-description').focus();
}

function hideAddTaskModal() {
    document.getElementById('add-task-modal').close();
}

function updateComposerAvailability() {
    var button = document.getElementById('task-submit-btn');
    var input = document.getElementById('task-description');
    var status = document.getElementById('task-submission-status');
    if (!button) return;
    var hasAgents = Object.values(AppState.agents).some(function(agent) { return agent.status === 'connected'; });
    var sending = !!taskSubmission && !taskSubmission.unknown;
    button.disabled = sending || AppState.connection !== 'connected' || snapshotState('swarm') !== 'current' || !hasAgents;
    button.textContent = taskSubmission && taskSubmission.unknown ? 'Send again' : 'Create task';
    input.readOnly = sending;
    if (sending) status.textContent = 'Sending · awaiting gateway acceptance';
    else if (taskSubmission) status.textContent = 'Outcome unknown · no receipt yet. Sending again is safe: the gateway accepts each draft once.';
    else if (AppState.connection !== 'connected') status.textContent = 'Not sent · gateway disconnected. Draft retained.';
    else if (snapshotState('swarm') !== 'current') status.textContent = 'Not sent · awaiting a current swarm snapshot';
    else if (!hasAgents) status.textContent = 'Not sent · no connected agents';
    else status.textContent = 'Draft · kept in this tab until accepted';
}

function submitTask() {
    if (taskSubmission && !taskSubmission.unknown) return;
    updateComposerAvailability();
    if (document.getElementById('task-submit-btn').disabled) return;
    var input = document.getElementById('task-description');
    var description = input.value.trim();
    if (!description) {
        document.getElementById('task-submission-status').textContent = 'Enter an objective';
        input.focus();
        return;
    }
    // The same unchanged draft keeps its id, so the gateway answers a resend
    // with the task it may already have opened instead of a second one.
    var requestId = taskSubmission && taskSubmission.description === description ? taskSubmission.requestId : newRequestId();
    if (!wsSend({ type: 'submitTask', description: description, requestId: requestId })) {
        document.getElementById('task-submission-status').textContent = 'Not sent · gateway disconnected. Draft retained.';
        return;
    }
    taskSubmission = { requestId: requestId, description: description, unknown: false };
    taskReceiptTimer = setTimeout(markSubmissionUnknown, 15000);
    updateComposerAvailability();
}

function markSubmissionUnknown() {
    if (!taskSubmission) return;
    taskSubmission.unknown = true;
    clearTimeout(taskReceiptTimer);
    updateComposerAvailability();
}

function acceptTaskSubmission(event) {
    if (!taskSubmission || event.requestId !== taskSubmission.requestId) return;
    clearTimeout(taskReceiptTimer);
    taskSubmission = null;
    document.getElementById('task-description').value = '';
    hideAddTaskModal();
    updateComposerAvailability();
    showToast('Task accepted by gateway', 'info');
}

window.addEventListener('beforeunload', function(event) {
    if (taskSubmission || document.getElementById('task-description').value.trim()) {
        event.preventDefault();
        event.returnValue = '';
    }
});
