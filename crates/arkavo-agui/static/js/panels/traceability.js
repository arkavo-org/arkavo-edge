"use strict";

var _traceHover = null;
var _tracePin = null;
var _traceDebounce = null;
var _scenarioRects = Object.create(null);
var _fileRects = Object.create(null);
var _scenarioLookup = null;
var _codeLookup = null;
var _scenarioStatusMap = null;
var _clipIdCounter = 0;

function renderTraceability() {
    var container = document.getElementById('traceability-container');
    if (!container) return;

    if (AppState.traceabilityData) {
        renderTraceabilityContent(container, AppState.traceabilityData);
        return;
    }

    container.innerHTML = '<div class="empty-state"><h3>Loading Traceability Data...</h3></div>';

    fetch('/static/data/traceability.json')
        .then(function(r) { return r.json(); })
        .then(function(data) {
            AppState.traceabilityData = data;
            renderTraceabilityContent(container, data);
        })
        .catch(function() {
            container.innerHTML = '<div class="empty-state"><h3>No Traceability Data</h3>' +
                '<p>Run: cargo xtask spec-test export-json</p></div>';
        });
}

function renderTraceabilityContent(container, data) {
    if (!data.specs || data.specs.length === 0) {
        container.innerHTML = '<div class="empty-state"><h3>No Traceability Data</h3>' +
            '<p>Run: cargo xtask spec-test export-json</p></div>';
        return;
    }

    _scenarioRects = Object.create(null);
    _fileRects = Object.create(null);
    _scenarioLookup = buildScenarioLookup(data.links);
    _codeLookup = buildCodeLookup(data.links);
    _scenarioStatusMap = buildScenarioStatusMap(data.specs);
    _clipIdCounter = 0;
    _tracePin = null;
    _traceHover = null;

    var html = renderSummaryStats(data.summary);
    html += '<div class="traceability-wrapper">';
    html += '<div class="treemap-container" id="spec-treemap"></div>';
    html += '<svg class="bridge-overlay" id="bridge-svg"></svg>';
    html += '<div class="treemap-container" id="code-treemap"></div>';
    html += '</div>';
    html += '<div class="traceability-detail" id="trace-detail"></div>';

    container.innerHTML = html;

    renderSpecTreemap(data.specs, document.getElementById('spec-treemap'));
    renderCodeTreemap(data.code, document.getElementById('code-treemap'));

    container.addEventListener('click', function(e) {
        var leaf = e.target.closest('.treemap-leaf');
        if (leaf) {
            var type = leaf.dataset.type;
            var id = leaf.dataset.id;
            if (_tracePin && _tracePin.type === type && _tracePin.id === id) {
                _tracePin = null;
                clearHighlight();
                hideDetail();
            } else {
                _tracePin = { type: type, id: id };
                applyHighlight(type, id);
                showDetail(type, id, data);
            }
            return;
        }
        if (!e.target.closest('.traceability-detail')) {
            _tracePin = null;
            clearHighlight();
            hideDetail();
        }
    });

    container.addEventListener('mouseover', function(e) {
        if (_tracePin) return;
        var leaf = e.target.closest('.treemap-leaf');
        if (!leaf) return;
        var type = leaf.dataset.type;
        var id = leaf.dataset.id;
        if (_traceHover && _traceHover.type === type && _traceHover.id === id) return;
        clearTimeout(_traceDebounce);
        _traceDebounce = setTimeout(function() {
            _traceHover = { type: type, id: id };
            applyHighlight(type, id);
        }, 50);
    });

    container.addEventListener('mouseout', function(e) {
        if (_tracePin) return;
        var leaf = e.target.closest('.treemap-leaf');
        if (!leaf) return;
        clearTimeout(_traceDebounce);
        _traceDebounce = setTimeout(function() {
            _traceHover = null;
            clearHighlight();
        }, 50);
    });
}

function renderSummaryStats(summary) {
    var pct = summary.pct.toFixed(1);
    var pctClass = summary.pct >= 50 ? 'slope-up' : summary.pct >= 25 ? 'slope-flat' : 'slope-down';
    return '<div class="summary-bar">' +
        '<div class="summary-stat"><div class="summary-stat-value">' + summary.total + '</div>' +
        '<div class="summary-stat-label">Scenarios</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value" style="color:var(--success)">' + summary.covered + '</div>' +
        '<div class="summary-stat-label">Covered</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value" style="color:var(--warning)">' + summary.partial + '</div>' +
        '<div class="summary-stat-label">Partial</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value" style="color:var(--error)">' + summary.missing + '</div>' +
        '<div class="summary-stat-label">Missing</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value ' + pctClass + '">' + pct + '%</div>' +
        '<div class="summary-stat-label">Coverage</div></div>' +
        '</div>';
}

function renderSpecTreemap(specs, container) {
    if (!container) return;

    // Build flat items: one per scenario, grouped by spec
    var groups = [];
    for (var i = 0; i < specs.length; i++) {
        var spec = specs[i];
        var items = [];
        for (var j = 0; j < spec.scenarios.length; j++) {
            var sc = spec.scenarios[j];
            items.push({
                value: sc.test_count + 1,
                id: sc.id,
                name: sc.name,
                status: sc.status,
                criticality: sc.criticality,
                specName: spec.name
            });
        }
        if (items.length > 0) {
            groups.push({ name: spec.name, items: items });
        }
    }

    var rect = container.getBoundingClientRect();
    var width = rect.width || 400;
    var height = Math.max(400, rect.height);

    // Layout groups first
    var groupItems = [];
    for (var g = 0; g < groups.length; g++) {
        var total = 0;
        for (var gi = 0; gi < groups[g].items.length; gi++) total += groups[g].items[gi].value;
        groupItems.push({ value: total, _group: groups[g] });
    }

    var groupRects = squarify(groupItems, { x: 0, y: 0, w: width, h: height });

    var svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + width + '" height="' + height + '" class="treemap-svg">';

    for (var gr = 0; gr < groupRects.length; gr++) {
        var grp = groupRects[gr];
        var group = grp._group;
        var gx = grp.x, gy = grp.y, gw = grp.w, gh = grp.h;
        var labelH = 16;

        // Group label background + clipped label
        svg += '<rect x="' + gx + '" y="' + gy + '" width="' + gw + '" height="' + labelH +
            '" fill="var(--bg-tertiary)" stroke="var(--border-color)" stroke-width="0.5"/>';
        var glClip = 'gc' + (_clipIdCounter++);
        svg += '<clipPath id="' + glClip + '"><rect x="' + gx + '" y="' + gy +
            '" width="' + gw + '" height="' + labelH + '"/></clipPath>';
        svg += '<text x="' + (gx + 4) + '" y="' + (gy + 11) +
            '" clip-path="url(#' + glClip + ')" class="treemap-group-label">' +
            escapeHtml(group.name) + '</text>';

        // Layout scenarios within group
        var innerRect = { x: gx + 1, y: gy + labelH, w: gw - 2, h: gh - labelH - 1 };
        if (innerRect.w > 0 && innerRect.h > 0) {
            var leafRects = squarify(group.items.slice(), innerRect);
            for (var lr = 0; lr < leafRects.length; lr++) {
                var leaf = leafRects[lr];
                var alpha = critAlpha(leaf.criticality);
                var color = statusColor(leaf.status);
                _scenarioRects[leaf.id] = { x: leaf.x, y: leaf.y, w: leaf.w, h: leaf.h };

                svg += '<rect class="treemap-leaf" data-type="scenario" data-id="' + escapeHtml(leaf.id) + '"' +
                    ' x="' + leaf.x + '" y="' + leaf.y + '" width="' + leaf.w + '" height="' + leaf.h + '"' +
                    ' fill="' + color + '" opacity="' + alpha + '"' +
                    ' stroke="var(--bg-primary)" stroke-width="1" rx="2"/>';

                if (leaf.w > 30 && leaf.h > 14) {
                    var lClip = 'lc' + (_clipIdCounter++);
                    svg += '<clipPath id="' + lClip + '"><rect x="' + leaf.x + '" y="' + leaf.y +
                        '" width="' + leaf.w + '" height="' + leaf.h + '"/></clipPath>';
                    svg += '<text x="' + (leaf.x + leaf.w / 2) + '" y="' + (leaf.y + leaf.h / 2 + 3) +
                        '" text-anchor="middle" clip-path="url(#' + lClip + ')"' +
                        ' class="treemap-leaf-label" pointer-events="none">' +
                        escapeHtml(leaf.id) + '</text>';
                }
            }
        }
    }

    svg += '</svg>';
    container.innerHTML = svg;
}

function renderCodeTreemap(code, container) {
    if (!container) return;

    var groups = [];
    for (var i = 0; i < code.length; i++) {
        var crate = code[i];
        var items = [];
        for (var j = 0; j < crate.files.length; j++) {
            var file = crate.files[j];
            items.push({
                value: Math.max(file.scenarios.length, 1),
                id: file.path,
                name: file.path.split('/').pop() || file.path,
                scenarioCount: file.scenarios.length,
                scenarios: file.scenarios,
                crateName: crate.name
            });
        }
        if (items.length > 0) {
            groups.push({ name: crate.name, items: items });
        }
    }

    if (groups.length === 0) {
        container.innerHTML = '<div class="empty-state"><p>No code linkages found</p></div>';
        return;
    }

    var rect = container.getBoundingClientRect();
    var width = rect.width || 400;
    var height = Math.max(400, rect.height);

    var groupItems = [];
    for (var g = 0; g < groups.length; g++) {
        var total = 0;
        for (var gi = 0; gi < groups[g].items.length; gi++) total += groups[g].items[gi].value;
        groupItems.push({ value: total, _group: groups[g] });
    }

    var groupRects = squarify(groupItems, { x: 0, y: 0, w: width, h: height });

    var svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + width + '" height="' + height + '" class="treemap-svg">';

    for (var gr = 0; gr < groupRects.length; gr++) {
        var grp = groupRects[gr];
        var group = grp._group;
        var gx = grp.x, gy = grp.y, gw = grp.w, gh = grp.h;
        var labelH = 16;

        svg += '<rect x="' + gx + '" y="' + gy + '" width="' + gw + '" height="' + labelH +
            '" fill="var(--bg-tertiary)" stroke="var(--border-color)" stroke-width="0.5"/>';
        var glClip2 = 'gc' + (_clipIdCounter++);
        svg += '<clipPath id="' + glClip2 + '"><rect x="' + gx + '" y="' + gy +
            '" width="' + gw + '" height="' + labelH + '"/></clipPath>';
        svg += '<text x="' + (gx + 4) + '" y="' + (gy + 11) +
            '" clip-path="url(#' + glClip2 + ')" class="treemap-group-label">' +
            escapeHtml(group.name) + '</text>';

        var innerRect = { x: gx + 1, y: gy + labelH, w: gw - 2, h: gh - labelH - 1 };
        if (innerRect.w > 0 && innerRect.h > 0) {
            var leafRects = squarify(group.items.slice(), innerRect);
            for (var lr = 0; lr < leafRects.length; lr++) {
                var leaf = leafRects[lr];
                _fileRects[leaf.id] = { x: leaf.x, y: leaf.y, w: leaf.w, h: leaf.h };

                var color = fileStatusColor(leaf.scenarios, _scenarioStatusMap);

                svg += '<rect class="treemap-leaf" data-type="file" data-id="' + escapeHtml(leaf.id) + '"' +
                    ' x="' + leaf.x + '" y="' + leaf.y + '" width="' + leaf.w + '" height="' + leaf.h + '"' +
                    ' fill="' + color + '" opacity="0.75"' +
                    ' stroke="var(--bg-primary)" stroke-width="1" rx="2"/>';

                if (leaf.w > 30 && leaf.h > 14) {
                    var fClip = 'fc' + (_clipIdCounter++);
                    svg += '<clipPath id="' + fClip + '"><rect x="' + leaf.x + '" y="' + leaf.y +
                        '" width="' + leaf.w + '" height="' + leaf.h + '"/></clipPath>';
                    svg += '<text x="' + (leaf.x + leaf.w / 2) + '" y="' + (leaf.y + leaf.h / 2 + 3) +
                        '" text-anchor="middle" clip-path="url(#' + fClip + ')"' +
                        ' class="treemap-leaf-label" pointer-events="none">' +
                        escapeHtml(leaf.name) + '</text>';
                }
            }
        }
    }

    svg += '</svg>';
    container.innerHTML = svg;
}

function applyHighlight(type, id) {
    // Dim all leaves
    var leaves = document.querySelectorAll('.treemap-leaf');
    for (var i = 0; i < leaves.length; i++) {
        leaves[i].style.opacity = '0.15';
    }

    var linkedIds = [];
    if (type === 'scenario') {
        // Highlight this scenario and linked files
        var el = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(id) + '"]');
        if (el) el.style.opacity = '1';
        linkedIds = (_scenarioLookup && _scenarioLookup[id]) || [];
        for (var j = 0; j < linkedIds.length; j++) {
            var fileEl = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(linkedIds[j]) + '"]');
            if (fileEl) fileEl.style.opacity = '1';
        }
    } else if (type === 'file') {
        var fileLeaf = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(id) + '"]');
        if (fileLeaf) fileLeaf.style.opacity = '1';
        linkedIds = (_codeLookup && _codeLookup[id]) || [];
        for (var k = 0; k < linkedIds.length; k++) {
            var scEl = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(linkedIds[k]) + '"]');
            if (scEl) scEl.style.opacity = '1';
        }
    }

    drawBridgeCurves(type, id, linkedIds);
}

function clearHighlight() {
    var leaves = document.querySelectorAll('.treemap-leaf');
    for (var i = 0; i < leaves.length; i++) {
        leaves[i].style.opacity = '';
    }
    var bridgeSvg = document.getElementById('bridge-svg');
    if (bridgeSvg) bridgeSvg.innerHTML = '';
}

function drawBridgeCurves(type, id, linkedIds) {
    var bridgeSvg = document.getElementById('bridge-svg');
    if (!bridgeSvg || linkedIds.length === 0) {
        if (bridgeSvg) bridgeSvg.innerHTML = '';
        return;
    }

    var wrapper = bridgeSvg.closest('.traceability-wrapper');
    if (!wrapper) return;
    var wrapperRect = wrapper.getBoundingClientRect();

    bridgeSvg.setAttribute('width', wrapperRect.width);
    bridgeSvg.setAttribute('height', wrapperRect.height);
    bridgeSvg.style.width = wrapperRect.width + 'px';
    bridgeSvg.style.height = wrapperRect.height + 'px';

    var paths = '';
    for (var i = 0; i < linkedIds.length; i++) {
        var fromRect, toRect;
        if (type === 'scenario') {
            fromRect = getLeafCenter('scenario', id, wrapperRect);
            toRect = getLeafCenter('file', linkedIds[i], wrapperRect);
        } else {
            fromRect = getLeafCenter('file', id, wrapperRect);
            toRect = getLeafCenter('scenario', linkedIds[i], wrapperRect);
        }
        if (!fromRect || !toRect) continue;

        var midX = (fromRect.cx + toRect.cx) / 2;
        paths += '<path class="bridge-curve" d="M ' + fromRect.cx + ',' + fromRect.cy +
            ' C ' + midX + ',' + fromRect.cy + ' ' + midX + ',' + toRect.cy +
            ' ' + toRect.cx + ',' + toRect.cy + '" />';
    }

    bridgeSvg.innerHTML = paths;
}

function getLeafCenter(type, id, wrapperRect) {
    var el = document.querySelector('.treemap-leaf[data-type="' + type + '"][data-id="' + CSS.escape(id) + '"]');
    if (!el) return null;
    var r = el.getBoundingClientRect();
    return {
        cx: r.left + r.width / 2 - wrapperRect.left,
        cy: r.top + r.height / 2 - wrapperRect.top
    };
}

function showDetail(type, id, data) {
    var detailEl = document.getElementById('trace-detail');
    if (!detailEl) return;

    if (type === 'scenario') {
        var scenario = findScenario(data, id);
        if (!scenario) { detailEl.innerHTML = ''; return; }
        var html = '<div class="trace-detail-card">';
        html += '<div class="trace-detail-header">' + escapeHtml(scenario.id) + ' - ' + escapeHtml(scenario.name) + '</div>';
        html += '<div class="trace-detail-meta">' + escapeHtml(scenario.criticality) + ' | ' + escapeHtml(scenario.status) +
            ' | ' + scenario.test_count + ' test(s)</div>';
        if (scenario.given.length > 0) {
            html += '<div class="trace-detail-section"><strong>Given:</strong>';
            for (var i = 0; i < scenario.given.length; i++) {
                html += '<div class="trace-detail-item">' + escapeHtml(scenario.given[i]) + '</div>';
            }
            html += '</div>';
        }
        html += '<div class="trace-detail-section"><strong>When:</strong> ' + escapeHtml(scenario.when) + '</div>';
        if (scenario.then.length > 0) {
            html += '<div class="trace-detail-section"><strong>Then:</strong>';
            for (var t = 0; t < scenario.then.length; t++) {
                html += '<div class="trace-detail-item">' + escapeHtml(scenario.then[t]) + '</div>';
            }
            html += '</div>';
        }
        if (scenario.refs.length > 0) {
            html += '<div class="trace-detail-section"><strong>Refs:</strong>';
            for (var r = 0; r < scenario.refs.length; r++) {
                html += '<div class="trace-detail-item" style="font-family:monospace;font-size:11px">' +
                    escapeHtml(scenario.refs[r]) + '</div>';
            }
            html += '</div>';
        }
        html += '</div>';
        detailEl.innerHTML = html;
    } else if (type === 'file') {
        var linkedScenarios = (_codeLookup && _codeLookup[id]) || [];
        var html2 = '<div class="trace-detail-card">';
        html2 += '<div class="trace-detail-header" style="font-family:monospace">' + escapeHtml(id) + '</div>';
        html2 += '<div class="trace-detail-meta">' + linkedScenarios.length + ' linked scenario(s)</div>';
        for (var s = 0; s < linkedScenarios.length; s++) {
            var sc = findScenario(data, linkedScenarios[s]);
            if (sc) {
                html2 += '<div class="trace-detail-item">' +
                    '<span style="color:' + statusColor(sc.status) + '">' + escapeHtml(sc.id) + '</span> ' +
                    escapeHtml(sc.name) + '</div>';
            }
        }
        html2 += '</div>';
        detailEl.innerHTML = html2;
    }
}

function hideDetail() {
    var detailEl = document.getElementById('trace-detail');
    if (detailEl) detailEl.innerHTML = '';
}

function buildScenarioStatusMap(specs) {
    var map = Object.create(null);
    for (var i = 0; i < specs.length; i++) {
        for (var j = 0; j < specs[i].scenarios.length; j++) {
            var sc = specs[i].scenarios[j];
            map[sc.id] = sc.status;
        }
    }
    return map;
}

function fileStatusColor(scenarioIds, statusMap) {
    if (!scenarioIds || scenarioIds.length === 0) return 'var(--text-secondary)';
    var covered = 0;
    for (var i = 0; i < scenarioIds.length; i++) {
        var st = statusMap && statusMap[scenarioIds[i]];
        if (st === 'covered' || st === 'partial') covered++;
    }
    var ratio = covered / scenarioIds.length;
    if (ratio >= 0.8) return 'var(--success)';
    if (ratio >= 0.4) return 'var(--warning)';
    if (ratio > 0) return 'var(--error)';
    return 'var(--error)';
}

function findScenario(data, id) {
    for (var i = 0; i < data.specs.length; i++) {
        for (var j = 0; j < data.specs[i].scenarios.length; j++) {
            if (data.specs[i].scenarios[j].id === id) {
                return data.specs[i].scenarios[j];
            }
        }
    }
    return null;
}
