"use strict";

var taskSubmission = null;
var taskReceiptTimer = null;

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
    button.disabled = !!taskSubmission || AppState.connection !== 'connected' || snapshotState('fleet') !== 'current' || !hasAgents;
    input.readOnly = !!taskSubmission;
    if (taskSubmission) status.textContent = taskSubmission.unknown ? 'Outcome unknown · receipt not received. Draft retained; check Tasks before resubmitting.' : 'Sending · awaiting gateway acceptance';
    else if (AppState.connection !== 'connected') status.textContent = 'Not sent · gateway disconnected. Draft retained.';
    else if (snapshotState('fleet') !== 'current') status.textContent = 'Not sent · awaiting a current fleet snapshot';
    else if (!hasAgents) status.textContent = 'Not sent · no connected agents';
    else status.textContent = 'Draft · kept in this tab until accepted';
}

function submitTask() {
    if (taskSubmission) return;
    updateComposerAvailability();
    if (document.getElementById('task-submit-btn').disabled) return;
    var input = document.getElementById('task-description');
    var description = input.value.trim();
    if (!description) {
        document.getElementById('task-submission-status').textContent = 'Enter an objective';
        input.focus();
        return;
    }
    var requestId = crypto.randomUUID();
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
