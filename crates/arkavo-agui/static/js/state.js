"use strict";

var AppState = {
    agents: {},
    tasks: {},
    telemetry: [],
    eventCount: 0,
    taskCount: 0,
    totalCost: 0,
    budgetStatus: null,
    roiDashboard: null,
    costMetrics: null,
    lastStatusUpdate: null,
    modelDecisions: [],
    securityStatus: null,
    tdfAuditLog: [],
    policyLog: [],
    activeView: 'agents',
    learningAgents: {},
    routingHistory: [],
    routingAnimations: [],
    pathWeights: Object.create(null),
    qualityTrends: [],
    lessons: [],
    lessonCount: 0,
    selectedTaskId: null,
    teachingEvents: [],
    agentSystemMetrics: {}
};

var MAX_TELEMETRY = 200;
var MAX_DECISIONS = 100;
var MAX_AUDIT_LOG = 200;
var MAX_ROUTING_HISTORY = 50;

function escapeHtml(text) {
    if (text == null) return '';
    var div = document.createElement('div');
    div.textContent = String(text);
    return div.innerHTML;
}

function formatTime(ts) {
    if (!ts) return '';
    var d = new Date(ts);
    return d.toLocaleTimeString();
}

function formatCost(dollars) {
    if (dollars == null || dollars === 0) return '$0.00';
    if (dollars < 0.01) return '<$0.01';
    return '$' + dollars.toFixed(2);
}
