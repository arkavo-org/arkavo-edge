"use strict";

// Notices agents bring to the operator (#759, AGUI-027 to AGUI-029). A
// notice can point at a place in the workspace; whether it takes the view
// there is the viewer's choice, and never while they are typing or have a
// dialog open: then it waits in the banner.

var NOTICE_FOCUS_KEY = 'arkavo.noticeFocus';
var noticeQueue = [];
// Notices this viewer hid from the banner; a reconnect lists them again.
var hiddenNotices = Object.create(null);

function noticeFocusSetting() {
    try {
        var value = localStorage.getItem(NOTICE_FOCUS_KEY);
        return value === 'all' || value === 'never' ? value : 'critical';
    } catch (e) {
        return 'critical';
    }
}

function setNoticeFocusSetting(value) {
    try { localStorage.setItem(NOTICE_FOCUS_KEY, value); } catch (e) { /* kept for this page only */ }
    renderNoticeDialog();
}

function noticeBusy() {
    var active = document.activeElement;
    var typing = active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' || active.tagName === 'SELECT' || active.isContentEditable);
    var dialogOpen = Array.prototype.some.call(document.querySelectorAll('dialog'), function(d) { return d.open; });
    return !!(typing || dialogOpen);
}

function takesFocus(notice) {
    var setting = noticeFocusSetting();
    if (!notice.focus || setting === 'never') return false;
    return setting === 'all' || notice.severity === 'critical';
}

function focusParams(focus) {
    return { agent: focus.agent || undefined, task: focus.task || undefined };
}

function openNoticeFocus(focus) {
    navigateRoute(focus.view, focusParams(focus));
}

function handleOperatorNotice(event) {
    var notice = event.notice;
    var id = notice && safeTaskKey(notice.id);
    if (!id) return;
    var isNew = !AppState.notices[id];
    // An update to a notice that already moved the view does not move it again.
    if (!isNew && AppState.notices[id]._opened) notice._opened = true;
    AppState.notices[id] = notice;
    if (isNew && notice.severity !== 'info') {
        noticeQueue = noticeQueue.filter(function(n) { return n !== id; });
        noticeQueue.push(id);
        if (takesFocus(notice)) {
            // The notice stays in the banner after it moves the view, so the
            // operator sees why the view changed.
            if (noticeBusy()) {
                showToast(notice.title + ' · waiting until you finish', 'warning');
            } else {
                notice._opened = true;
                openNoticeFocus(notice.focus);
            }
        }
    }
    addTelemetry({ eventType: 'operator-notice', agentId: notice.agentId, details: { noticeId: notice.id, severity: notice.severity, title: notice.title }, timestamp: notice.raisedAt });
    renderNotices();
}

function handleNoticeResolved(event) {
    var id = safeTaskKey(event.noticeId);
    if (!id) return;
    delete AppState.notices[id];
    noticeQueue = noticeQueue.filter(function(n) { return n !== id; });
    renderNotices();
}

function handleNoticeList(event) {
    var previous = AppState.notices;
    AppState.notices = Object.create(null);
    (event.notices || []).slice().sort(function(a, b) {
        return (Date.parse(a.raisedAt) || 0) - (Date.parse(b.raisedAt) || 0);
    }).forEach(function(notice) {
        var id = safeTaskKey(notice.id);
        if (!id) return;
        // A notice raised before this view connected waits in the banner
        // like one that arrives now, but never moves the view.
        notice._opened = previous[id] ? !!previous[id]._opened : true;
        AppState.notices[id] = notice;
        if (notice.severity !== 'info' && !hiddenNotices[id] && noticeQueue.indexOf(id) < 0) noticeQueue.push(id);
    });
    noticeQueue = noticeQueue.filter(function(id) { return AppState.notices[id]; });
    renderNotices();
}

function handleNoticeActionResult(event) {
    delete AppState.noticeActionsPending[event.noticeId + '/' + event.actionId];
    if (event.error) showToast('Not done · ' + event.error, 'warning');
    else if (event.actionId === 'retry') showToast('Task submitted again', 'info');
    renderNotices();
}

function sortedNotices() {
    var rank = { critical: 0, warning: 1, info: 2 };
    return Object.values(AppState.notices).sort(function(a, b) {
        return (rank[a.severity] - rank[b.severity]) || (Date.parse(b.raisedAt) || 0) - (Date.parse(a.raisedAt) || 0);
    });
}

function noticeActions(notice) {
    return (notice.actions || []).map(function(action) {
        var pending = AppState.noticeActionsPending[notice.id + '/' + action.id];
        return '<button type="button" class="notice-action' + (action.kind === 'acknowledge' ? ' quiet' : '') + '" data-notice="' + escapeHtml(notice.id) +
            '" data-notice-agent="' + escapeHtml(notice.agentId) + '" data-notice-action="' + escapeHtml(action.id) + '"' +
            (pending || (action.kind !== 'focus' && AppState.connection !== 'connected') ? ' disabled' : '') + '>' +
            escapeHtml(pending ? 'Sending' : action.label) + '</button>';
    }).join('');
}

function noticeCard(notice) {
    var details = (notice.details || []).map(function(d) {
        return '<dt>' + escapeHtml(d.label) + '</dt><dd>' + escapeHtml(d.value) + '</dd>';
    }).join('');
    return '<article class="notice-card severity-' + escapeHtml(notice.severity) + '"><header>' +
        uiIcon(notice.severity === 'info' ? 'bell' : 'triangle-alert') + '<strong>' + escapeHtml(notice.title) + '</strong>' +
        '<span class="notice-meta">' + escapeHtml(notice.agentId) + (notice.count > 1 ? ' · ' + escapeHtml(String(notice.count)) + ' times' : '') +
        ' · <span class="age" data-time="' + escapeHtml(notice.raisedAt) + '">' + observationAge(notice.raisedAt) + '</span></span></header>' +
        '<p>' + escapeHtml(notice.text) + '</p>' + (details ? '<dl>' + details + '</dl>' : '') +
        '<div class="notice-actions">' + noticeActions(notice) + '</div></article>';
}

function renderNotices() {
    var notices = sortedNotices();
    var button = document.getElementById('notices-toggle');
    if (button) {
        var alerts = notices.filter(function(n) { return n.severity !== 'info'; }).length;
        button.innerHTML = uiIcon('bell') + '<span class="sr-only">Notices</span>' + (notices.length ? '<span class="notice-count' + (alerts ? ' alert' : '') + '">' + notices.length + '</span>' : '');
        button.setAttribute('aria-label', notices.length + ' open notice' + (notices.length === 1 ? '' : 's'));
    }
    var banner = document.getElementById('notice-banner');
    if (banner) {
        var waiting = noticeQueue.map(function(id) { return AppState.notices[id]; }).filter(Boolean);
        var top = waiting[waiting.length - 1];
        // The Overview lists open notices under Needs attention already.
        if (AppState.activeView === 'overview') top = null;
        banner.hidden = !top;
        banner.className = 'notice-banner' + (top ? ' severity-' + top.severity : '');
        banner.innerHTML = top ? noticeCard(top) + '<button type="button" class="notice-hide" data-notice-hide="' + escapeHtml(top.id) + '">Hide</button>' : '';
    }
    renderNoticeDialog();
    scheduleViewRender();
}

function renderNoticeDialog() {
    var list = document.getElementById('notices-list');
    if (!list) return;
    var notices = sortedNotices();
    var setting = noticeFocusSetting();
    var options = [['critical', 'Critical notices'], ['all', 'Every notice'], ['never', 'Never']];
    list.innerHTML = '<label class="notice-setting">Open where a notice points: <select id="notice-focus-setting">' +
        options.map(function(o) { return '<option value="' + o[0] + '"' + (o[0] === setting ? ' selected' : '') + '>' + o[1] + '</option>'; }).join('') +
        '</select></label>' + (notices.length ? notices.map(noticeCard).join('') : '<div class="ops-empty"><strong>No open notices</strong>Agents raise notices when something needs you.</div>');
}

function overviewNotices() {
    var notices = sortedNotices().slice(0, 3);
    return notices.map(noticeCard).join('');
}

function chooseNoticeAction(button) {
    var notice = AppState.notices[button.dataset.notice];
    if (!notice) return;
    var action = (notice.actions || []).find(function(a) { return a.id === button.dataset.noticeAction; });
    if (!action) return;
    if (action.kind === 'focus') {
        document.getElementById('notices-dialog').close();
        openNoticeFocus(action.focus);
        noticeQueue = noticeQueue.filter(function(n) { return n !== notice.id; });
    } else {
        AppState.noticeActionsPending[notice.id + '/' + action.id] = true;
    }
    // Every choice goes to the gateway, which keeps a record of it and
    // carries out the ones that leave the browser.
    if (!wsSend({ type: 'noticeAction', agentId: notice.agentId, noticeId: notice.id, actionId: action.id }) && action.kind !== 'focus') {
        delete AppState.noticeActionsPending[notice.id + '/' + action.id];
        showToast('Not sent · gateway disconnected', 'warning');
    }
    renderNotices();
}

function initializeNotices() {
    document.addEventListener('click', function(e) {
        var action = e.target.closest && e.target.closest('[data-notice-action]');
        if (action) chooseNoticeAction(action);
        var hide = e.target.closest && e.target.closest('[data-notice-hide]');
        if (hide) {
            hiddenNotices[hide.dataset.noticeHide] = true;
            noticeQueue = noticeQueue.filter(function(n) { return n !== hide.dataset.noticeHide; });
            renderNotices();
        }
        if (e.target.closest && e.target.closest('#notices-toggle')) document.getElementById('notices-dialog').showModal();
        if (e.target.closest && e.target.closest('#notices-close')) document.getElementById('notices-dialog').close();
    });
    document.addEventListener('change', function(e) {
        if (e.target && e.target.id === 'notice-focus-setting') setNoticeFocusSetting(e.target.value);
    });
    // A notice that waited while the operator typed opens once they stop.
    document.addEventListener('focusout', function() {
        setTimeout(function() {
            if (noticeBusy()) return;
            var waiting = noticeQueue.map(function(id) { return AppState.notices[id]; }).filter(function(n) { return n && takesFocus(n) && !n._opened; });
            var next = waiting[waiting.length - 1];
            if (!next) return;
            next._opened = true;
            openNoticeFocus(next.focus);
            renderNotices();
        }, 0);
    });
    renderNotices();
}
