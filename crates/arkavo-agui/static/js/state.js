"use strict";

var AppState = {
    agents: Object.create(null),
    tasks: Object.create(null),
    connection: 'reconnecting',
    observations: Object.create(null),
    telemetry: [],
    eventCount: 0,
    evictedEvents: 0,
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
    dataPlaneStatus: null,
    dataPlaneTransfers: [],
    activeView: 'overview',
    learningAgents: {},
    routingHistory: [],
    routingAnimations: [],
    pathWeights: Object.create(null),
    qualityTrends: [],
    lessons: [],
    lessonCount: 0,
    selectedTaskId: null,
    selectedAgentId: null,
    route: null,
    teachingEvents: [],
    agentSystemMetrics: {},
    optimalConfigs: [],
    metricsHistory: [],
    lastMetricsSnapshot: null,
    contextTopology: null,
    selectedContextAgent: null,
    conversationWindow: null,
    arpStatus: null,
    arpSelectedAgent: null,
    lastPublishedTrust: null,
    publishedTrustLastFingerprint: null,
    publishedTrustSelectedAgent: null,
};

var MAX_TELEMETRY = 200;
var MAX_METRICS_HISTORY = 60;
var MAX_DECISIONS = 100;
var MAX_AUDIT_LOG = 200;
var MAX_TRANSFERS = 200;
var MAX_ROUTING_HISTORY = 50;

function escapeHtml(text) {
    if (text == null) return '';
    return String(text).replace(/[&<>"']/g, function(character) {
        return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[character];
    });
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
