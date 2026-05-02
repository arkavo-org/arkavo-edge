"use strict";

function routeEvent(event) {
    // Notify showcase mode of incoming events for intelligent view switching
    if (typeof showcaseOnEvent === 'function') showcaseOnEvent(event);

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
        case 'computeBudgetUpdate':
            handleComputeBudgetUpdate(event);
            break;
        case 'agentSystemMetrics':
            handleAgentSystemMetrics(event);
            break;
        case 'securityStatusUpdate':
            handleSecurityStatusUpdate(event);
            break;
        case 'tdfAuditEvent':
            handleTdfAuditEvent(event);
            break;
        case 'policyApplied':
            handlePolicyApplied(event);
            break;
        case 'dataPlaneStatusUpdate':
            handleDataPlaneStatusUpdate(event);
            break;
        case 'dataPlaneTransfer':
            handleDataPlaneTransfer(event);
            break;
        case 'modelSelected':
            handleModelSelected(event);
            break;
        case 'learningStatusUpdate':
            handleLearningStatusUpdate(event);
            break;
        case 'routingEvaluation':
            handleRoutingEvaluation(event);
            break;
        case 'routingOutcome':
            handleRoutingOutcome(event);
            break;
        case 'lessonExtracted':
            handleLessonExtracted(event);
            break;
        case 'contextTopologyUpdate':
            handleContextTopologyUpdate(event);
            break;
        case 'arpStatusUpdate':
            handleArpStatusUpdate(event);
            break;
        case 'publishedTrustUpdate':
            handlePublishedTrustUpdate(event);
            break;
        case 'systemNotification':
            handleSystemNotification(event);
            break;
        case 'error':
            handleChatError(event);
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
    } else if (viewId === 'security') {
        requestSecurityData();
        requestDataPlaneData();
    } else if (viewId === 'learning' || viewId === 'router') {
        wsSend({ type: 'requestLearningStatus' });
        startLearningPolling();
    } else if (viewId === 'context') {
        wsSend({ type: 'requestContextTopology' });
        startContextPolling();
    } else if (viewId === 'arp') {
        wsSend({ type: 'requestArpStatus' });
        startArpPolling();
    } else if (viewId === 'published-trust') {
        wsSend({ type: 'requestPublishedTrust' });
        startPublishedTrustPolling();
    }

    // Stop polling when navigating away
    if (viewId !== 'learning' && viewId !== 'router') {
        stopLearningPolling();
    }
    if (viewId !== 'context') {
        stopContextPolling();
    }
    if (viewId !== 'arp') {
        stopArpPolling();
    }
    if (viewId !== 'published-trust') {
        stopPublishedTrustPolling();
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

var showcaseBtn = document.getElementById('showcase-toggle');
if (showcaseBtn) showcaseBtn.addEventListener('click', function() {
    if (typeof showcaseToggle === 'function') showcaseToggle();
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
