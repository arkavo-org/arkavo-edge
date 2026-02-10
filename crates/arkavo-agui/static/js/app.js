"use strict";

function routeEvent(event) {
    switch (event.type) {
        case 'meshStatus':
            handleMeshStatus(event);
            break;
        case 'agentDiscovered':
            addAgent(event);
            break;
        case 'agentLost':
            removeAgent(event);
            break;
        case 'a2aMessage':
            addA2AMessage(event);
            break;
        case 'telemetryEvent':
            addTelemetry(event);
            break;
        case 'statusUpdate':
            handleStatusUpdate(event);
            break;
        case 'taskList':
            handleTaskList(event);
            break;
        case 'taskSubmitted':
            handleTaskSubmitted(event);
            break;
        case 'taskStatusChanged':
            handleTaskStatusChanged(event);
            break;
        case 'messagesSnapshot':
            handleMessagesSnapshot(event);
            break;
        case 'messageDelta':
            handleMessageDelta(event);
            break;
        case 'budgetStatusUpdate':
            handleBudgetStatusUpdate(event);
            break;
        case 'budgetAlert':
            handleBudgetAlert(event);
            break;
        case 'spendingRecorded':
            handleSpendingRecorded(event);
            break;
        case 'rOIDashboardUpdate':
            handleROIDashboardUpdate(event);
            break;
        case 'costMetricsUpdate':
            handleCostMetricsUpdate(event);
            break;
        case 'modelSelected':
            handleModelSelected(event);
            break;
        case 'systemNotification':
            handleSystemNotification(event);
            break;
        default:
            if (event.type) {
                addTelemetry({
                    eventType: event.type,
                    agentId: event.agentId || 'system',
                    details: event,
                    timestamp: new Date().toISOString()
                });
            }
    }
}

function updateStats() {
    document.getElementById('event-count').textContent = AppState.eventCount;
    document.getElementById('task-count').textContent = Object.keys(AppState.tasks).length;
}

function switchView(viewId) {
    AppState.activeView = viewId;

    // Update nav buttons
    var btns = document.querySelectorAll('.nav-btn');
    for (var i = 0; i < btns.length; i++) {
        btns[i].classList.toggle('active', btns[i].dataset.view === viewId);
    }

    // Update view panels
    var panels = document.querySelectorAll('.view-panel');
    for (var j = 0; j < panels.length; j++) {
        panels[j].classList.toggle('active', panels[j].id === 'view-' + viewId);
    }

    // Request data for panels that need it
    if (viewId === 'budget') {
        requestBudgetData();
    }
}

function showToast(message, severity) {
    var container = document.getElementById('toast-container');
    if (!container) return;

    var toast = document.createElement('div');
    toast.className = 'toast ' + (severity || 'info');
    toast.textContent = message;
    container.appendChild(toast);

    setTimeout(function() {
        toast.style.opacity = '0';
        setTimeout(function() { toast.remove(); }, 300);
    }, 5000);
}

// Bind event listeners (CSP blocks inline onclick)
document.querySelectorAll('.nav-btn').forEach(function(btn) {
    btn.addEventListener('click', function() { switchView(btn.dataset.view); });
});

var addTaskBtn = document.getElementById('add-task-btn');
if (addTaskBtn) addTaskBtn.addEventListener('click', showAddTaskModal);

var taskCancelBtn = document.getElementById('task-cancel-btn');
if (taskCancelBtn) taskCancelBtn.addEventListener('click', hideAddTaskModal);

var taskSubmitBtn = document.getElementById('task-submit-btn');
if (taskSubmitBtn) taskSubmitBtn.addEventListener('click', submitTask);

// Initialize
connect();
switchView('agents');
