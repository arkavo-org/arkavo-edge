"use strict";

function routeEvent(event) {
    var snapshot = { meshStatus: 'fleet', taskList: 'tasks', arpStatusUpdate: 'policy' }[event.type];
    if (snapshot) AppState.observations[snapshot] = Date.now();
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
        case 'arpProposalActionResult':
            handlePolicyReviewResult(event);
            break;
        case 'flightStopped':
            handleFlightStopped(event);
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
    if (AppState.activeView === 'overview') renderOverview();
    if (AppState.activeView === 'decisions') renderDecisions();
    if (AppState.activeView === 'tasks' && AppState.selectedTaskId) renderTaskDetail();
    if (AppState.activeView === 'agents' && AppState.selectedAgentId) renderAgentDetail();
    updateComposerAvailability();
}

function updateStats() {
    document.getElementById('event-count').textContent = AppState.eventCount;
    document.getElementById('task-count').textContent = Object.keys(AppState.tasks).length;
}

function activateView(viewId) {
    if (!document.getElementById('view-' + viewId)) return;
    AppState.activeView = viewId;

    // Update nav buttons
    updateNavigation(viewId);

    // Update view panels
    var panels = document.querySelectorAll('.view-panel');
    for (var j = 0; j < panels.length; j++) {
        panels[j].classList.toggle('active', panels[j].id === 'view-' + viewId);
    }

    // Request data for panels that need it
    if (viewId === 'overview' || viewId === 'decisions') {
        wsSend({ type: 'requestArpStatus' });
        wsSend({ type: 'requestTaskList' });
        if (viewId === 'overview') renderOverview();
        else renderDecisions();
    } else if (viewId === 'tasks') {
        wsSend({ type: 'requestTaskList' });
        if (AppState.selectedTaskId) {
            wsSend({ type: 'requestLearningStatus' });
            wsSend({ type: 'requestArpStatus' });
        }
        renderTasks();
    } else if (viewId === 'agents') {
        wsSend({ type: 'requestFleetSnapshot' });
        if (AppState.selectedAgentId) {
            wsSend({ type: 'requestTaskList' });
            wsSend({ type: 'requestArpStatus' });
        }
        renderAgents();
        renderAgentDetail();
    } else if (viewId === 'telemetry') {
        renderEvidence();
    } else if (viewId === 'diagnostics') {
        renderTelemetry();
    } else if (viewId === 'budget') {
        requestBudgetData();
    } else if (viewId === 'security') {
        wsSend({ type: 'requestMeshStatus' });
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
        renderArp();
        startArpPolling();
    }

    // Published Trust is rendered inside the Security & Data Plane panel,
    // so its polling is gated on entering the security view.
    if (viewId === 'security') {
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
    if (viewId !== 'security') {
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
initializeOperations();
connect();
initializeRoutes();
