"use strict";

var _pendingTaskDescription = null;

function handleTaskList(event) {
    AppState.tasks = {};
    if (event.tasks) {
        event.tasks.forEach(function(task) {
            AppState.tasks[task.id] = task;
        });
    }
    renderTasks();
}

function handleTaskSubmitted(event) {
    var task = {
        id: event.taskId || event.task_id,
        description: _pendingTaskDescription || 'Task',
        status: event.status || 'submitted',
        created_at: event.timestamp
    };
    _pendingTaskDescription = null;
    AppState.tasks[task.id] = task;
    AppState.taskCount++;
    renderTasks();
    updateStats();
    addTelemetry({
        eventType: 'task-submitted',
        agentId: 'system',
        details: { taskId: task.id },
        timestamp: event.timestamp
    });
}

function handleTaskStatusChanged(event) {
    var id = event.taskId || event.task_id;
    if (AppState.tasks[id]) {
        AppState.tasks[id].status = event.status;
        if (event.progress !== undefined) {
            AppState.tasks[id].progress = event.progress;
        }
        if (event.result) {
            AppState.tasks[id].result = event.result;
        }
        if (event.status === 'completed') {
            AppState.tasks[id].completed_at = event.timestamp;
        }
    }
    renderTasks();
    addTelemetry({
        eventType: 'task-' + event.status,
        agentId: event.targetAgent || 'system',
        details: { taskId: id, progress: event.progress },
        timestamp: event.timestamp
    });
}

function renderTasks() {
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
        var targetAgent = task.target_agent || task.targetAgent;
        var progress = typeof task.progress === 'number' ? Math.min(100, Math.max(0, task.progress * 100)) : null;
        return '<div class="task-card">' +
            '<div class="task-header">' +
            '<span class="task-id">#' + taskId + '</span>' +
            '<span class="task-status ' + statusClass + '">' + status + '</span>' +
            '</div>' +
            '<div class="task-description">' + (escapeHtml(task.description) || 'Task') + '</div>' +
            '<div class="task-meta">' +
            (targetAgent ? 'Agent: ' + escapeHtml(targetAgent) : 'Auto-assigned') +
            ((task.created_at || task.createdAt) ? ' | ' + escapeHtml(formatTime(task.created_at || task.createdAt)) : '') +
            '</div>' +
            (progress !== null ? '<div class="task-progress"><div class="task-progress-bar" style="width: ' + progress + '%"></div></div>' : '') +
            '</div>';
    }).join('');

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
