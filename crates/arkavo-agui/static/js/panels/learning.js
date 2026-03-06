"use strict";

// Cortical Routing Map - Learning Panel

var CONNECTOME_SIZE = 380;
var CONNECTOME_CENTER = CONNECTOME_SIZE / 2;
var ORBIT_RADIUS = 140;
var NODE_MIN = 16;
var NODE_MAX = 50;
var selectedLearningAgent = null;
var learningPollInterval = null;

function startLearningPolling() {
    stopLearningPolling();
    learningPollInterval = setInterval(function() {
        if (AppState.activeView === 'learning' || AppState.activeView === 'router') {
            wsSend({ type: 'requestLearningStatus' });
        } else {
            stopLearningPolling();
        }
    }, 10000);
}

function stopLearningPolling() {
    if (learningPollInterval) {
        clearInterval(learningPollInterval);
        learningPollInterval = null;
    }
}

function handleLearningStatusUpdate(event) {
    var agents = event.agents || [];
    AppState.learningAgents = {};
    for (var i = 0; i < agents.length; i++) {
        AppState.learningAgents[agents[i].agentId] = agents[i];
    }
    AppState.routingHistory = event.routingHistory || [];
    AppState.qualityTrends = event.qualityTrends || [];
    AppState.lessons = event.lessons || AppState.lessons || [];
    AppState.lessonCount = AppState.lessons.length || event.lessonCount || 0;
    renderLearningPanel();

    // Keep polling while on the learning panel
    if (AppState.activeView === 'learning' && !learningPollInterval) {
        startLearningPolling();
    }
}

function handleRoutingEvaluation(event) {
    // Update agent scores from candidates
    var candidates = event.candidates || [];
    for (var i = 0; i < candidates.length; i++) {
        var c = candidates[i];
        AppState.learningAgents[c.agentId] = AppState.learningAgents[c.agentId] || {};
        var a = AppState.learningAgents[c.agentId];
        a.agentId = c.agentId;
        a.alpha = c.alpha;
        a.betaParam = c.betaParam;
        a.observations = c.observations;
        a.totalObservations = c.observations;
        a.successRate = c.successRate;
        a.probationary = c.probationary;
        a.expectedValue = c.alpha / (c.alpha + c.betaParam);
        a.stdDev = Math.sqrt((c.alpha * c.betaParam) / ((c.alpha + c.betaParam) * (c.alpha + c.betaParam) * (c.alpha + c.betaParam + 1)));
    }

    // Track path weight for selected agent
    var sel = event.selectedAgent;
    AppState.pathWeights[sel] = (AppState.pathWeights[sel] || 0) + 1;

    // Write selected agent back to task object for detail view
    if (event.taskId && AppState.tasks[event.taskId]) {
        AppState.tasks[event.taskId].target_agent = sel;
    }

    // Add to routing history
    AppState.routingHistory.push({
        taskId: event.taskId,
        selectedAgent: sel,
        wasExploration: event.wasExploration,
        outcome: null,
        category: event.category || null,
        timestamp: event.timestamp
    });
    while (AppState.routingHistory.length > MAX_ROUTING_HISTORY) {
        AppState.routingHistory.shift();
    }

    renderLearningPanel();
    animateRouting(event);
}

function handleRoutingOutcome(event) {
    // Update routing history with outcome and quality
    var quality = (typeof event.qualityScore === 'number') ? event.qualityScore : (event.success ? 1.0 : 0.0);
    var issues = event.qualityIssues || [];

    for (var i = AppState.routingHistory.length - 1; i >= 0; i--) {
        if (AppState.routingHistory[i].taskId === event.taskId) {
            if (event.success && quality > 0.5) {
                AppState.routingHistory[i].outcome = 'success';
            } else if (event.success) {
                AppState.routingHistory[i].outcome = 'degraded';
            } else {
                AppState.routingHistory[i].outcome = 'failed';
            }
            AppState.routingHistory[i].qualityScore = quality;
            AppState.routingHistory[i].qualityIssues = issues;
            break;
        }
    }

    animateOutcome(event);
    renderTimeline();
}

function renderLearningPanel() {
    var container = document.getElementById('learning-container');
    if (!container) return;

    var agentIds = Object.keys(AppState.learningAgents);
    if (agentIds.length === 0 && AppState.routingHistory.length === 0) {
        container.innerHTML = '<div class="empty-state"><h3>Waiting for Routing Data</h3><p>Submit tasks to watch the mesh brain think</p></div>';
        return;
    }

    container.innerHTML =
        renderSummaryBar() +
        renderQualityChart() +
        '<div class="learning-layout">' +
            '<div class="connectome-area">' +
                '<svg id="connectome-svg" class="connectome-svg" viewBox="0 0 ' + CONNECTOME_SIZE + ' ' + CONNECTOME_SIZE + '"></svg>' +
            '</div>' +
            '<div class="beta-sidebar" id="beta-sidebar">' +
                renderBetaCard(selectedLearningAgent) +
            '</div>' +
        '</div>' +
        '<div class="routing-timeline-section">' +
            '<div class="timeline-header">Routing Timeline</div>' +
            '<div class="routing-timeline" id="routing-timeline"></div>' +
        '</div>' +
        '<div class="lesson-feed-section">' +
            '<div class="timeline-header">Lessons Learned</div>' +
            '<div class="lesson-feed" id="lesson-feed"></div>' +
        '</div>' +
        renderTeachingSection();

    renderConnectome(agentIds);
    renderTimeline();
    renderLessonFeed();
}

function renderConnectome(agentIds) {
    var svg = document.getElementById('connectome-svg');
    if (!svg) return;

    // Filter out model names that may appear as agent IDs
    // Model names contain patterns like "8b", "27b", "0.8b", "3.5", "flash"
    agentIds = agentIds.filter(function(id) {
        if (id.indexOf('.gguf') !== -1 || id.indexOf('/') !== -1) return false;
        if (/\d+[\.\d]*b$/i.test(id)) return false;
        if (/^\w+-\d+\.\d+/.test(id)) return false;
        if (/-(flash|turbo|pro|ultra|nano|micro|mini|base|large|medium|small)$/i.test(id)) return false;
        return true;
    });

    var ns = 'http://www.w3.org/2000/svg';
    svg.innerHTML = '';

    // Draw center hub
    var hubG = document.createElementNS(ns, 'g');
    hubG.setAttribute('class', 'hub-node');

    var hubCircle = document.createElementNS(ns, 'circle');
    hubCircle.setAttribute('cx', CONNECTOME_CENTER);
    hubCircle.setAttribute('cy', CONNECTOME_CENTER);
    hubCircle.setAttribute('r', '22');
    hubCircle.setAttribute('class', 'hub-circle');
    hubG.appendChild(hubCircle);

    var hubLabel = document.createElementNS(ns, 'text');
    hubLabel.setAttribute('x', CONNECTOME_CENTER);
    hubLabel.setAttribute('y', CONNECTOME_CENTER + 4);
    hubLabel.setAttribute('class', 'hub-label');
    hubLabel.textContent = 'Gateway';
    hubG.appendChild(hubLabel);
    svg.appendChild(hubG);

    // Draw agent nodes in a circle
    var count = agentIds.length;
    for (var i = 0; i < count; i++) {
        var angle = (2 * Math.PI * i / count) - Math.PI / 2;
        var x = CONNECTOME_CENTER + ORBIT_RADIUS * Math.cos(angle);
        var y = CONNECTOME_CENTER + ORBIT_RADIUS * Math.sin(angle);
        var agent = AppState.learningAgents[agentIds[i]];
        var obs = (agent && agent.totalObservations) || 0;
        var sr = (agent && agent.successRate) || 0;
        var probationary = agent && agent.probationary;

        // Path from center to node
        var pathWeight = AppState.pathWeights[agentIds[i]] || 0;
        var strokeWidth = 2 + Math.log2(1 + pathWeight);

        var path = document.createElementNS(ns, 'line');
        path.setAttribute('x1', CONNECTOME_CENTER);
        path.setAttribute('y1', CONNECTOME_CENTER);
        path.setAttribute('x2', x);
        path.setAttribute('y2', y);
        path.setAttribute('class', 'routing-path');
        path.setAttribute('id', 'path-' + agentIds[i]);
        path.setAttribute('stroke-width', strokeWidth);
        svg.appendChild(path);

        // Node size based on observations
        var nodeR = NODE_MIN + Math.min(obs / 5, NODE_MAX - NODE_MIN);

        // Node color based on success rate
        var color = successRateColor(sr);

        var g = document.createElementNS(ns, 'g');
        g.setAttribute('class', 'agent-node' + (probationary ? ' probationary' : ''));
        g.setAttribute('id', 'node-' + agentIds[i]);
        g.setAttribute('data-agent', agentIds[i]);

        var circle = document.createElementNS(ns, 'circle');
        circle.setAttribute('cx', x);
        circle.setAttribute('cy', y);
        circle.setAttribute('r', nodeR);
        circle.setAttribute('fill', color);
        circle.setAttribute('fill-opacity', '0.25');
        circle.setAttribute('stroke', color);
        circle.setAttribute('stroke-width', probationary ? '2' : '2.5');
        if (probationary) {
            circle.setAttribute('stroke-dasharray', '4 3');
        }
        g.appendChild(circle);

        // Agent label
        var label = document.createElementNS(ns, 'text');
        label.setAttribute('x', x);
        label.setAttribute('y', y + nodeR + 14);
        label.setAttribute('class', 'node-label');
        label.textContent = shortAgentName(agentIds[i]);
        g.appendChild(label);

        // Model label below agent name
        var agentInfo = AppState.agents[agentIds[i]];
        var modelName = (agentInfo && agentInfo.model) ? shortModelName(agentInfo.model)
            : (agent && agent.model ? shortModelName(agent.model) : '');
        if (modelName) {
            var modelLabel = document.createElementNS(ns, 'text');
            modelLabel.setAttribute('x', x);
            modelLabel.setAttribute('y', y + nodeR + 26);
            modelLabel.setAttribute('class', 'node-model');
            modelLabel.textContent = modelName;
            g.appendChild(modelLabel);
        }

        // EV text inside node
        var evText = document.createElementNS(ns, 'text');
        evText.setAttribute('x', x);
        evText.setAttribute('y', y + 4);
        evText.setAttribute('class', 'node-ev');
        var ev = agent ? agent.expectedValue : 0.667;
        evText.textContent = ev.toFixed(2);
        g.appendChild(evText);

        // Per-agent lesson count badge
        var agentLessonCount = 0;
        for (var li = 0; li < (AppState.lessons || []).length; li++) {
            if (AppState.lessons[li].agentId === agentIds[i]) agentLessonCount++;
        }
        if (agentLessonCount > 0) {
            var bx = x + nodeR * 0.7, by = y - nodeR * 0.7;
            var bc = document.createElementNS(ns, 'circle');
            bc.setAttribute('cx', bx);
            bc.setAttribute('cy', by);
            bc.setAttribute('r', '8');
            bc.setAttribute('class', 'lesson-badge-circle');
            g.appendChild(bc);
            var bt = document.createElementNS(ns, 'text');
            bt.setAttribute('x', bx);
            bt.setAttribute('y', by + 3);
            bt.setAttribute('class', 'lesson-badge-text');
            bt.textContent = agentLessonCount;
            g.appendChild(bt);
        }

        // Click handler
        (function(aid) {
            g.addEventListener('click', function() { selectLearningAgent(aid); });
        })(agentIds[i]);

        svg.appendChild(g);
    }
}

function animateRouting(event) {
    var candidates = event.candidates || [];

    // Flash all candidate nodes
    for (var i = 0; i < candidates.length; i++) {
        var node = document.getElementById('node-' + candidates[i].agentId);
        if (node) {
            node.classList.add('evaluating');
            (function(n) {
                setTimeout(function() { n.classList.remove('evaluating'); }, 1200);
            })(node);
        }
    }

    // Illuminate selected path
    var path = document.getElementById('path-' + event.selectedAgent);
    if (path) {
        path.classList.add('selected');
        if (event.wasExploration) {
            path.classList.add('exploring');
        }
        setTimeout(function() {
            path.classList.remove('selected');
            path.classList.remove('exploring');
        }, 2500);
    }

    // Flash selected node
    var selNode = document.getElementById('node-' + event.selectedAgent);
    if (selNode) {
        selNode.classList.add('selected-node');
        setTimeout(function() { selNode.classList.remove('selected-node'); }, 2500);
    }
}

function animateOutcome(event) {
    var path = document.getElementById('path-' + event.agentId);
    if (!path) return;

    var quality = (typeof event.qualityScore === 'number') ? event.qualityScore : (event.success ? 1.0 : 0.0);

    if (event.success && quality > 0.5) {
        // Good outcome: green glow
        path.classList.add('success');
        setTimeout(function() { path.classList.remove('success'); }, 1500);
    } else if (event.success) {
        // Degraded: task "completed" but judge says low quality
        path.classList.add('degraded');
        setTimeout(function() {
            path.classList.remove('degraded');
            path.classList.add('scarred');
        }, 1500);
    } else {
        // Failure: red scar
        path.classList.add('failure');
        setTimeout(function() {
            path.classList.remove('failure');
            path.classList.add('scarred');
        }, 1500);
    }
}

function renderTimeline() {
    var tl = document.getElementById('routing-timeline');
    if (!tl) return;

    var html = '';
    var history = AppState.routingHistory;
    for (var i = history.length - 1; i >= 0 && i >= history.length - 20; i--) {
        var r = history[i];
        var dotClass = 'route-dot';
        if (r.outcome === 'success') dotClass += ' success';
        else if (r.outcome === 'degraded') dotClass += ' degraded';
        else if (r.outcome === 'failed') dotClass += ' failure';
        else if (r.wasExploration) dotClass += ' exploration';
        else if (r.outcome === null) dotClass += ' pending';
        else dotClass += ' exploitation';

        // Quality badge for completed tasks
        var qualityBadge = '';
        if (r.qualityScore !== undefined && r.qualityScore !== null && r.outcome) {
            var qClass = r.qualityScore > 0.7 ? 'quality-good' : (r.qualityScore > 0.3 ? 'quality-degraded' : 'quality-poor');
            qualityBadge = ' <span class="quality-badge ' + qClass + '">' + (r.qualityScore * 100).toFixed(0) + '%</span>';
        }

        // Category badge — infer from agent context when router returns "general"
        var displayCat = r.category;
        if ((!displayCat || displayCat === 'general') && inferredDomainCategory()) {
            displayCat = inferredDomainCategory();
        }
        var categoryBadge = '';
        if (displayCat) {
            categoryBadge = ' <span class="category-badge">' + escapeHtml(shortCategory(displayCat)) + '</span>';
        }

        // Quality issues tooltip
        var issueText = '';
        if (r.qualityIssues && r.qualityIssues.length > 0) {
            issueText = ' title="' + escapeHtml(r.qualityIssues.join('; ')) + '"';
        }

        html += '<div class="route-entry"' + issueText + '>' +
            '<span class="' + dotClass + '"></span>' +
            '<span class="route-agent">' + escapeHtml(shortAgentName(r.selectedAgent)) + '</span>' +
            categoryBadge +
            qualityBadge +
            '<span class="route-time">' + formatTime(r.timestamp) + '</span>' +
            '</div>';
    }

    tl.innerHTML = html || '<span class="empty-timeline">No routing decisions yet</span>';
}

function renderBetaCard(agentId) {
    if (!agentId || !AppState.learningAgents[agentId]) {
        return '<div class="beta-card-empty">Click an agent node to see its Beta distribution</div>';
    }

    var a = AppState.learningAgents[agentId];
    var alpha = a.alpha || 2;
    var beta = a.betaParam || 1;

    // Build beta curve SVG
    var curvePoints = [];
    var maxY = 0;
    var steps = 80;
    for (var i = 0; i <= steps; i++) {
        var x = i / steps;
        if (x <= 0.001) x = 0.001;
        if (x >= 0.999) x = 0.999;
        var y = betaPdf(x, alpha, beta);
        if (y > maxY) maxY = y;
        curvePoints.push({ x: x, y: y });
    }

    var svgW = 220;
    var svgH = 100;
    var pad = 10;
    var plotW = svgW - 2 * pad;
    var plotH = svgH - 2 * pad;

    var pathD = 'M ' + pad + ' ' + (svgH - pad);
    for (var j = 0; j < curvePoints.length; j++) {
        var px = pad + curvePoints[j].x * plotW;
        var py = (svgH - pad) - (curvePoints[j].y / (maxY || 1)) * plotH;
        pathD += ' L ' + px.toFixed(1) + ' ' + py.toFixed(1);
    }
    pathD += ' L ' + (svgW - pad) + ' ' + (svgH - pad) + ' Z';

    var color = successRateColor(a.successRate || 0);

    // Per-category stats table
    var catHtml = '';
    var catStats = a.categoryStats || [];
    if (catStats.length > 0) {
        catHtml = '<div class="category-breakdown"><div class="category-breakdown-title">Per-Category</div>' +
            '<table class="category-table"><thead><tr><th>Category</th><th>E[V]</th><th>Obs</th></tr></thead><tbody>';
        for (var ci = 0; ci < catStats.length; ci++) {
            var cs = catStats[ci];
            var evClass = cs.expectedValue > 0.7 ? 'ev-good' : (cs.expectedValue > 0.4 ? 'ev-mid' : 'ev-bad');
            catHtml += '<tr><td>' + escapeHtml(shortCategory(cs.category)) + '</td>' +
                '<td class="' + evClass + '">' + cs.expectedValue.toFixed(3) + '</td>' +
                '<td>' + cs.observations + '</td></tr>';
        }
        catHtml += '</tbody></table></div>';
    }

    // Quality trends for this agent
    var trendsHtml = renderQualityTrends(agentId);

    // Lesson details for this agent
    var agentLessons = (AppState.lessons || []).filter(function(l) { return l.agentId === agentId; });
    var lessonBadge = '';
    if (agentLessons.length > 0) {
        lessonBadge = '<div class="lesson-list-section"><div class="category-breakdown-title">Lessons (' + agentLessons.length + ')</div>';
        for (var li = 0; li < agentLessons.length && li < 5; li++) {
            var lesson = agentLessons[li];
            lessonBadge += '<div class="lesson-item">' +
                '<div class="lesson-condition">' + escapeHtml(sanitizeCondition(lesson.condition)) + '</div>' +
                '<div class="lesson-action">' + escapeHtml(lesson.action) + '</div>' +
                '<div class="lesson-meta">' + escapeHtml(shortCategory(lesson.category)) +
                    ' | conf: ' + lesson.confidence.toFixed(2) + '</div>' +
                '</div>';
        }
        lessonBadge += '</div>';
    } else if (AppState.lessonCount > 0) {
        lessonBadge = '<div class="lesson-count-badge">' + AppState.lessonCount + ' lessons cached</div>';
    }

    return '<div class="beta-card">' +
        '<div class="beta-card-title">' + escapeHtml(shortAgentName(agentId)) + '</div>' +
        '<svg class="beta-svg" viewBox="0 0 ' + svgW + ' ' + svgH + '">' +
            '<path d="' + pathD + '" fill="' + color + '" fill-opacity="0.3" stroke="' + color + '" stroke-width="2" />' +
            '<line x1="' + pad + '" y1="' + (svgH - pad) + '" x2="' + (svgW - pad) + '" y2="' + (svgH - pad) + '" stroke="var(--border-color)" stroke-width="1" />' +
            '<text x="' + pad + '" y="' + (svgH - 1) + '" class="axis-label">0</text>' +
            '<text x="' + (svgW - pad) + '" y="' + (svgH - 1) + '" class="axis-label" text-anchor="end">1</text>' +
        '</svg>' +
        '<div class="beta-stats">' +
            '<div class="beta-stat"><span class="beta-stat-label">Alpha</span><span class="beta-stat-value">' + alpha.toFixed(1) + '</span></div>' +
            '<div class="beta-stat"><span class="beta-stat-label">Beta</span><span class="beta-stat-value">' + beta.toFixed(1) + '</span></div>' +
            '<div class="beta-stat"><span class="beta-stat-label">E[V]</span><span class="beta-stat-value">' + (a.expectedValue || 0).toFixed(3) + '</span></div>' +
            '<div class="beta-stat"><span class="beta-stat-label">Obs</span><span class="beta-stat-value">' + (a.totalObservations || 0) + '</span></div>' +
            '<div class="beta-stat"><span class="beta-stat-label">Success</span><span class="beta-stat-value">' + ((a.successRate || 0) * 100).toFixed(0) + '%</span></div>' +
            (a.probationary ? '<div class="beta-probation">Probationary</div>' : '<div class="beta-graduated">Graduated</div>') +
        '</div>' +
        catHtml +
        trendsHtml +
        lessonBadge +
    '</div>';
}



function selectLearningAgent(agentId) {
    selectedLearningAgent = agentId;
    var sidebar = document.getElementById('beta-sidebar');
    if (sidebar) {
        sidebar.innerHTML = renderBetaCard(agentId);
    }
}

// Beta PDF approximation
function betaPdf(x, a, b) {
    if (x <= 0 || x >= 1) return 0;
    return Math.exp((a - 1) * Math.log(x) + (b - 1) * Math.log(1 - x) - lnBeta(a, b));
}

function lnBeta(a, b) {
    return lnGamma(a) + lnGamma(b) - lnGamma(a + b);
}

// Stirling's approximation for ln(Gamma(x))
function lnGamma(x) {
    if (x <= 0) return 0;
    if (x < 0.5) {
        // Reflection formula
        return Math.log(Math.PI / Math.sin(Math.PI * x)) - lnGamma(1 - x);
    }
    x -= 1;
    var g = 7;
    var c = [
        0.99999999999980993, 676.5203681218851, -1259.1392167224028,
        771.32342877765313, -176.61502916214059, 12.507343278686905,
        -0.13857109526572012, 9.9843695780195716e-6, 1.5056327351493116e-7
    ];
    var t = c[0];
    for (var i = 1; i < g + 2; i++) {
        t += c[i] / (x + i);
    }
    var w = x + g + 0.5;
    return 0.5 * Math.log(2 * Math.PI) + (x + 0.5) * Math.log(w) - w + Math.log(t);
}



function inferredDomainCategory() {
    // If connected agents belong to a specific domain, infer that for "general" records
    var agentIds = Object.keys(AppState.agents || {});
    if (agentIds.length === 0) return null;
    var gameCount = 0;
    for (var i = 0; i < agentIds.length; i++) {
        var id = agentIds[i].toLowerCase();
        if (id.indexOf('rimworld') !== -1 || id.indexOf('colony') !== -1 || id.indexOf('game') !== -1) {
            gameCount++;
        }
    }
    if (gameCount > 0 && gameCount >= agentIds.length / 2) {
        return 'game_simulation';
    }
    return null;
}

function shortAgentName(id) {
    if (!id) return 'unknown';
    // Remove common prefixes, truncate long names
    var name = id.replace(/^arkavo-/, '').replace(/-agent$/, '');
    if (name.length > 16) name = name.substring(0, 14) + '..';
    return name;
}

function shortCategory(cat) {
    if (!cat) return '';
    // Convert snake_case to short display name
    var map = {
        'frontend_ui': 'frontend',
        'backend_api': 'backend',
        'code_search': 'search',
        'security_scan': 'security',
        'test_generation': 'tests',
        'documentation': 'docs',
        'refactoring': 'refactor',
        'code_generation': 'codegen',
        'vision_analysis': 'vision',
        'game_simulation': 'game',
        'general': 'general'
    };
    return map[cat] || cat;
}

function handleLessonExtracted(event) {
    AppState.lessons.unshift({
        agentId: event.agentId,
        category: event.category,
        condition: event.condition,
        action: event.action,
        confidence: event.confidence,
        timestamp: event.timestamp
    });
    if (AppState.lessons.length > 50) AppState.lessons.pop();
    AppState.lessonCount = AppState.lessons.length;

    animateLesson(event);
    renderLessonFeed();
    var sidebar = document.getElementById('beta-sidebar');
    if (sidebar && selectedLearningAgent === event.agentId) {
        sidebar.innerHTML = renderBetaCard(event.agentId);
    }
}

function animateLesson(event) {
    var node = document.getElementById('node-' + event.agentId);
    if (!node) return;
    node.classList.add('lesson-pulse');
    setTimeout(function() { node.classList.remove('lesson-pulse'); }, 2000);
}

function lessonFailureType(condition) {
    if (condition.indexOf('empty responses') !== -1) return 'empty';
    if (condition.indexOf('generic non-answers') !== -1) return 'generic';
    if (condition.indexOf('overly brief') !== -1) return 'too_short';
    if (condition.indexOf('tool call failures') !== -1) return 'tool_error';
    if (condition.indexOf('hallucinated') !== -1) return 'hallucinated';
    if (condition.indexOf('output loops') !== -1) return 'output_loop';
    return condition.substring(0, 30);
}

function sanitizeCondition(text) {
    if (!text) return '';
    // Strip "(task: ...)" suffix that may contain JSON noise
    var taskIdx = text.indexOf(' (task: ');
    if (taskIdx !== -1) {
        var taskPart = text.substring(taskIdx + 7);
        // Remove JSON fragments from task part
        taskPart = taskPart.replace(/\{[^}]*\}/g, '').replace(/\s+/g, ' ').trim();
        taskPart = taskPart.replace(/\)$/, '');
        if (taskPart.length > 60) taskPart = taskPart.substring(0, 58) + '..';
        text = text.substring(0, taskIdx) + (taskPart ? ' (' + taskPart + ')' : '');
    }
    if (text.length > 120) text = text.substring(0, 118) + '..';
    return text;
}

function renderLessonFeed() {
    var feed = document.getElementById('lesson-feed');
    if (!feed) return;
    var lessons = AppState.lessons || [];
    if (lessons.length === 0) {
        feed.innerHTML = '<span class="empty-timeline">No lessons extracted yet</span>';
        return;
    }

    // Deduplicate by (agentId, failureType) — keep highest confidence, count occurrences
    var groups = {};
    for (var i = 0; i < lessons.length; i++) {
        var l = lessons[i];
        var key = l.agentId + '|' + lessonFailureType(l.condition);
        if (!groups[key] || l.confidence > groups[key].confidence) {
            groups[key] = { agentId: l.agentId, category: l.category, condition: l.condition, action: l.action, confidence: l.confidence, timestamp: l.timestamp, count: (groups[key] ? groups[key].count : 0) + 1 };
        } else {
            groups[key].count++;
        }
    }

    // Sort by confidence descending
    var deduped = [];
    var keys = Object.keys(groups);
    for (var j = 0; j < keys.length; j++) deduped.push(groups[keys[j]]);
    deduped.sort(function(a, b) { return b.confidence - a.confidence; });

    var html = '';
    var limit = Math.min(deduped.length, 10);
    for (var k = 0; k < limit; k++) {
        var d = deduped[k];
        var catBadge = d.category ? '<span class="category-badge">' + escapeHtml(shortCategory(d.category)) + '</span>' : '';
        var confClass = d.confidence > 0.7 ? 'conf-high' : 'conf-mid';
        var countBadge = d.count > 1 ? '<span class="lesson-count">x' + d.count + '</span>' : '';
        html += '<div class="lesson-feed-item">' +
            '<span class="route-agent">' + escapeHtml(shortAgentName(d.agentId)) + '</span>' +
            countBadge +
            catBadge +
            '<span class="lesson-feed-condition">' + escapeHtml(sanitizeCondition(d.condition)) + '</span>' +
            '<span class="lesson-conf ' + confClass + '">' + d.confidence.toFixed(2) + '</span>' +
            '<span class="route-time">' + formatTime(d.timestamp) + '</span>' +
            '</div>';
    }
    feed.innerHTML = html;
}

function shortModelName(model) {
    if (!model) return '';
    var name = model.split('/').pop().replace('.gguf', '');
    if (name.length > 18) name = name.substring(0, 16) + '..';
    return name;
}

function renderTeachingSection() {
    var events = AppState.teachingEvents || [];
    if (events.length === 0) return '';

    var html = '<div class="teaching-section">' +
        '<div class="timeline-header">Human Teaching</div>' +
        '<div class="teaching-event-list">';

    for (var i = events.length - 1; i >= 0; i--) {
        var evt = events[i];
        var intent = evt.intent || 'unknown';
        var badgeClass = 'teaching-badge';
        var icon = '';
        if (intent.indexOf('instruction') === 0) {
            badgeClass += ' instruction';
            icon = '&#x1F4DD;';
        } else if (intent === 'correction') {
            badgeClass += ' correction';
            icon = '&#x26D4;';
        } else if (intent === 'reinforcement') {
            badgeClass += ' reinforcement';
            icon = '&#x2705;';
        }

        var preview = escapeHtml((evt.text || '').substring(0, 60));
        if ((evt.text || '').length > 60) preview += '...';

        html += '<div class="teaching-event-item">' +
            '<span class="' + badgeClass + '">' +
                '<span class="teaching-icon">' + icon + '</span>' +
                '<span class="teaching-label">' + escapeHtml(intent) + '</span>' +
            '</span>' +
            '<span class="teaching-event-text">' + preview + '</span>' +
            '<span class="teaching-event-time">' + formatTime(evt.timestamp) + '</span>' +
            '</div>';
    }

    html += '</div></div>';
    return html;
}
