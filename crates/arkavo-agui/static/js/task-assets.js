"use strict";

// Images a task's agent delivered, shown with the task (AGUI-025). The
// gateway fetched and verified them; each URL serves only a file it
// recorded on the task.

function assetImage(asset, taskId, compact) {
    var size = escapeHtml(asset.width + '×' + asset.height);
    var figure = '<figure class="task-asset' + (asset.hasAlpha ? ' has-alpha' : '') + '">' +
        '<a href="' + escapeHtml(compact ? routeHref('tasks', { task: taskId }) : asset.url) + '"' + (compact ? '' : ' target="_blank" rel="noopener"') + '>' +
        '<img src="' + escapeHtml(asset.url) + '" alt="' + escapeHtml(asset.name + ', ' + size) + '" loading="lazy" decoding="async"></a>';
    if (!compact) {
        figure += '<figcaption><strong>' + escapeHtml(asset.name) + '</strong><span>' + size + ' · ' +
            (asset.hasAlpha ? 'transparent' : 'opaque') + ' · ' + escapeHtml(formatBytes(asset.bytes)) + '</span>' +
            '<small>' + escapeHtml(asset.license) + '</small></figcaption>';
    }
    return figure + '</figure>';
}

function formatBytes(bytes) {
    if (!Number.isFinite(bytes) || bytes < 0) return 'size not reported';
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1048576) return (bytes / 1024).toFixed(0) + ' KB';
    return (bytes / 1048576).toFixed(1) + ' MB';
}

function taskGallery(task) {
    var assets = task.assets || [];
    var html = assets.length ? '<div class="task-gallery">' + assets.map(function(asset) { return assetImage(asset, task.id, false); }).join('') + '</div>' : '';
    if (task.assetNote) html += '<p class="asset-note" role="status">' + escapeHtml(task.assetNote) + '</p>';
    return html;
}

function latestImages(tasks) {
    var shown = [];
    tasks.forEach(function(task) {
        (task.assets || []).forEach(function(asset) {
            if (shown.length < 6) shown.push(assetImage(asset, task.id, true));
        });
    });
    if (!shown.length) return '';
    return '<section class="ops-section"><div class="ops-section-heading"><h2>Latest images</h2><button data-navigate="tasks">All tasks</button></div>' +
        '<div class="task-gallery compact">' + shown.join('') + '</div></section>';
}
