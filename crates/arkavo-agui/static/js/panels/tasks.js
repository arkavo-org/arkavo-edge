"use strict";

var MAX_TASK_EVENTS = 50;

function addTaskEvent(taskId, evt) {
    var safeKey = safeTaskKey(taskId);
    if (!safeKey || !AppState.tasks[safeKey]) return;
    taskId = safeKey;
    var task = AppState.tasks[taskId];
    if (!task._events) task._events = [];
    task._events.unshift(evt);
    if (task._events.length > MAX_TASK_EVENTS) task._events.pop();
}

function safeTaskKey(key) {
    if (typeof key !== 'string' || !key || key === '__proto__' || key === 'constructor' || key === 'prototype') return null;
    return key;
}

function handleTaskList(event) {
    var previous = AppState.tasks;
    AppState.tasks = Object.create(null);
    if (event.tasks) {
        event.tasks.forEach(function(task) {
            var key = safeTaskKey(task.id);
            if (key) {
                // Older gateways omit output; session metrics and events also survive polling.
                var merged = Object.assign({}, previous[key] || {}, task);
                merged._events = previous[key] ? previous[key]._events : [];
                merged.completed_at = task.completed_at || task.completedAt || merged.completed_at;
                AppState.tasks[key] = merged;
            }
        });
    }
    renderTasks();
}

function handleTaskSubmitted(event) {
    var task = {
        id: event.taskId || event.task_id,
        description: event.description || 'Task',
        summary: event.summary || null,
        status: event.status || 'submitted',
        source: event.source || 'ui',
        created_at: event.timestamp,
        _events: []
    };
    var safeId = safeTaskKey(task.id);
    if (!safeId) return;
    acceptTaskSubmission(event);
    AppState.tasks[safeId] = task;
    AppState.taskCount++;

    var evt = {
        eventType: 'task-submitted',
        agentId: 'system',
        details: { taskId: task.id },
        timestamp: event.timestamp
    };
    addTaskEvent(task.id, evt);
    addTelemetry(evt);

    renderTasks();
    updateStats();
}

function handleTaskStatusChanged(event) {
    var id = safeTaskKey(event.taskId || event.task_id);
    if (id && AppState.tasks[id]) {
        AppState.tasks[id].status = event.status;
        if (event.progress !== undefined) {
            AppState.tasks[id].progress = event.progress;
        }
        if (event.result) {
            AppState.tasks[id].result = event.result;
        }
        if (event.metrics) {
            AppState.tasks[id].metrics = event.metrics;
        }
        if (event.assets || event.assetNote !== undefined) {
            AppState.tasks[id].assets = event.assets || AppState.tasks[id].assets || [];
            AppState.tasks[id].assetNote = event.assetNote || null;
        }
        if (event.status === 'completed') {
            AppState.tasks[id].completed_at = event.timestamp;
        }
    }

    var evt = {
        eventType: 'task-' + event.status,
        agentId: event.targetAgent || 'system',
        details: { taskId: id, progress: event.progress },
        timestamp: event.timestamp
    };
    addTaskEvent(id, evt);
    addTelemetry(evt);

    // If viewing task detail, refresh that; otherwise update card in-place
    if (AppState.selectedTaskId) {
        renderTaskDetail();
        return;
    }
    if (!updateTaskCardInPlace(id)) {
        renderTasks();
    }
}

// The task list leaves outputs out; a view that opens a task asks for it.
function handleTaskDetail(event) {
    var id = safeTaskKey(event.taskId);
    var task = id && AppState.tasks[id];
    if (!task) return;
    task.result = event.result || task.result;
    task.assets = event.assets || task.assets || [];
    task.assetNote = event.assetNote || null;
    task._detailRequested = false;
    if (AppState.selectedTaskId === id) renderTaskDetail();
}

function updateTaskCardInPlace(taskId) {
    if (!taskId) return false;
    var row = document.getElementById('tasks-container').querySelector('[data-task-row="' + CSS.escape(taskId) + '"]');
    var task = AppState.tasks[taskId];
    if (!row || !task) return false;
    var cell = row.querySelector('.task-row-state');
    cell.innerHTML = uiState(task.status);
    if (task.status === 'input-required' || task.status === 'auth-required') {
        cell.innerHTML += '<span class="row-secondary">' + (task.status === 'input-required' ? 'Input required' : 'Authorization required') + '</span>';
    } else if (task.status === 'failed' || task.status === 'blocked') {
        cell.innerHTML += '<span class="row-secondary">' + escapeHtml(task.result ? String(task.result).slice(0, 160) : 'Reason not reported') + '</span>';
    }
    return true;
}

function renderTasks() {
    if (AppState.selectedTaskId) {
        renderTaskDetail();
        return;
    }
    var container = document.getElementById('tasks-container');
    if (!container) return;
    delete container.dataset.detailFingerprint;
    var tasks = sortedTasks();
    var fingerprint = taskListFingerprint(tasks);
    if (container.dataset.fingerprint === fingerprint) return;
    container.dataset.fingerprint = fingerprint;
    container.innerHTML = tasks.length ? '<section class="ops-section">' + taskRows(tasks) + '</section>' :
        '<div class="ops-empty"><strong>No tasks reported</strong>Create a task when an agent is available.</div>';
    document.getElementById('task-count').textContent = tasks.length;
}
