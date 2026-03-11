"use strict";

var _pendingTaskDescription = null;
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
    if (key === '__proto__' || key === 'constructor' || key === 'prototype') return null;
    return key;
}

function handleTaskList(event) {
    AppState.tasks = Object.create(null);
    if (event.tasks) {
        event.tasks.forEach(function(task) {
            var key = safeTaskKey(task.id);
            if (key) AppState.tasks[key] = task;
        });
    }
    renderTasks();
}

function handleTaskSubmitted(event) {
    var task = {
        id: event.taskId || event.task_id,
        description: event.description || _pendingTaskDescription || 'Task',
        summary: event.summary || null,
        status: event.status || 'submitted',
        source: event.source || 'ui',
        created_at: event.timestamp,
        _events: []
    };
    if (task.source === 'ui') _pendingTaskDescription = null;
    var safeId = safeTaskKey(task.id);
    if (safeId) AppState.tasks[safeId] = task;
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

function updateTaskCardInPlace(taskId) {
    var card = document.querySelector('.task-card-clickable[data-task-id="' + CSS.escape(taskId) + '"]');
    if (!card) return false;
    var task = AppState.tasks[taskId];
    if (!task) return false;

    var statusEl = card.querySelector('.task-status');
    if (statusEl) {
        var status = task.status || '';
        statusEl.textContent = status;
        statusEl.className = 'task-status ' + status.replace(/[^a-zA-Z0-9-]/g, '');
    }

    var progressBar = card.querySelector('.task-progress-bar');
    if (progressBar && typeof task.progress === 'number') {
        var pct = Math.min(100, Math.max(0, task.progress * 100));
        progressBar.style.width = pct + '%';
    } else if (!progressBar && typeof task.progress === 'number') {
        var wrapper = document.createElement('div');
        wrapper.className = 'task-progress';
        var bar = document.createElement('div');
        bar.className = 'task-progress-bar';
        bar.style.width = Math.min(100, Math.max(0, task.progress * 100)) + '%';
        wrapper.appendChild(bar);
        card.appendChild(wrapper);
    }
    return true;
}

function selectTask(taskId) {
    AppState.selectedTaskId = taskId;
    renderTaskDetail();
}

function deselectTask() {
    AppState.selectedTaskId = null;
    renderTasks();
}

function renderTasks() {
    if (AppState.selectedTaskId) {
        renderTaskDetail();
        return;
    }
    var container = document.getElementById('tasks-container');
    if (!container) return;
    var tasks = Object.values(AppState.tasks);

    if (tasks.length === 0) {
        container.innerHTML =
            '<div class="empty-state">' +
            '<h3>No Tasks</h3>' +
            '<p>Click "+ Add Task" to submit work</p>' +
            '</div>';
        return;
    }

    tasks.sort(function(a, b) {
        return new Date(b.created_at || b.createdAt) - new Date(a.created_at || a.createdAt);
    });

    container.innerHTML = tasks.map(function(task) {
        var taskId = escapeHtml((task.id || '').slice(0, 8));
        var status = escapeHtml(task.status || '');
        var statusClass = status.replace(/[^a-zA-Z0-9-]/g, '');
        var agent = getTaskAgent(task, task.id);
        var agentDisplay = agent ? shortAgentName(agent) : 'Routing...';
        var progress = typeof task.progress === 'number' ? Math.min(100, Math.max(0, task.progress * 100)) : null;
        var sourceBadge = task.source === 'agent' ? '<span class="source-badge agent">auto</span> ' : '';
        return '<div class="task-card task-card-clickable" data-task-id="' + escapeHtml(task.id) + '">' +
            '<div class="task-header">' +
            sourceBadge + '<span class="task-id">#' + taskId + '</span>' +
            '<span class="task-status ' + statusClass + '">' + status + '</span>' +
            '</div>' +
            '<div class="task-description">' + escapeHtml(task.summary || task.description || 'Task') + '</div>' +
            '<div class="task-meta">' +
            escapeHtml(agentDisplay) +
            ((task.created_at || task.createdAt) ? ' | ' + escapeHtml(formatTime(task.created_at || task.createdAt)) : '') +
            '</div>' +
            (progress !== null ? '<div class="task-progress"><div class="task-progress-bar" style="width:' + progress + '%"></div></div>' : '') +
            '</div>';
    }).join('');

    var cards = container.querySelectorAll('.task-card-clickable');
    for (var i = 0; i < cards.length; i++) {
        (function(card) {
            card.addEventListener('click', function() {
                selectTask(card.getAttribute('data-task-id'));
            });
        })(cards[i]);
    }

    document.getElementById('task-count').textContent = tasks.length;
}

function showAddTaskModal() {
    document.getElementById('add-task-modal').classList.add('active');
    document.getElementById('task-description').focus();
}

function hideAddTaskModal() {
    document.getElementById('add-task-modal').classList.remove('active');
    document.getElementById('task-description').value = '';
}

function submitTask() {
    var description = document.getElementById('task-description').value.trim();
    if (!description) {
        alert('Please enter a task description');
        return;
    }
    _pendingTaskDescription = description;
    wsSend({ type: 'submitTask', description: description });
    hideAddTaskModal();
}
