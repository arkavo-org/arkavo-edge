"use strict";

function escapeHtml(text) {
    if (text == null) return '';
    var div = document.createElement('div');
    div.textContent = String(text);
    return div.innerHTML;
}

var _traceHover = null;
var _tracePin = null;
var _traceDebounce = null;
var _scenarioRects = Object.create(null);
var _fileRects = Object.create(null);
var _scenarioLookup = null;
var _codeLookup = null;
var _scenarioStatusMap = null;
var _clipIdCounter = 0;

function renderTraceabilityContent(container, data) {
    if (!data.specs || data.specs.length === 0) {
        container.innerHTML = '<div class="empty-state"><h3>No Traceability Data</h3></div>';
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

    html += '<div class="treemap-tooltip" id="treemap-tooltip"></div>';

    container.innerHTML = html;

    var tooltip = document.getElementById('treemap-tooltip');

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
        var target = e.target.closest('[data-tooltip]');
        if (target && tooltip) {
            tooltip.textContent = target.dataset.tooltip;
            tooltip.style.display = 'block';
        }
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

    container.addEventListener('mousemove', function(e) {
        if (tooltip && tooltip.style.display === 'block') {
            tooltip.style.left = (e.clientX + 12) + 'px';
            tooltip.style.top = (e.clientY + 12) + 'px';
        }
    });

    container.addEventListener('mouseout', function(e) {
        var target = e.target.closest('[data-tooltip]');
        if (target && tooltip) {
            tooltip.style.display = 'none';
        }
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
        '<div class="summary-stat"><div class="summary-stat-value" style="color:#22c55e">' + summary.covered + '</div>' +
        '<div class="summary-stat-label">Covered</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value" style="color:#eab308">' + summary.partial + '</div>' +
        '<div class="summary-stat-label">Partial</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value" style="color:#ef4444">' + summary.missing + '</div>' +
        '<div class="summary-stat-label">Missing</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value" style="color:#a855f7">' + (summary.wip || 0) + '</div>' +
        '<div class="summary-stat-label">WIP</div></div>' +
        '<div class="summary-stat"><div class="summary-stat-value ' + pctClass + '">' + pct + '%</div>' +
        '<div class="summary-stat-label">Coverage</div></div>' +
        '</div>';
}

function renderSpecTreemap(specs, container) {
    if (!container) return;

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
                specName: spec.name,
                wip: sc.wip || false
            });
        }
        if (items.length > 0) {
            groups.push({ name: spec.name, items: items });
        }
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

    var svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + width + '" height="' + height + '" class="treemap-svg">' +
        '<defs><pattern id="wip-stripes" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">' +
        '<line x1="0" y1="0" x2="0" y2="6" stroke="#0a0a0f" stroke-width="2"/></pattern></defs>';

    for (var gr = 0; gr < groupRects.length; gr++) {
        var grp = groupRects[gr];
        var group = grp._group;
        var gx = grp.x, gy = grp.y, gw = grp.w, gh = grp.h;
        var labelH = 16;

        svg += '<rect x="' + gx + '" y="' + gy + '" width="' + gw + '" height="' + labelH +
            '" fill="#1a1a2e" stroke="#2a2a3e" stroke-width="0.5"' +
            ' data-tooltip="' + escapeHtml(group.name) + ' (' + group.items.length + ' scenarios)"' +
            ' data-group-name="' + escapeHtml(group.name) + '"/>';
        var glClip = 'gc' + (_clipIdCounter++);
        svg += '<clipPath id="' + glClip + '"><rect x="' + gx + '" y="' + gy +
            '" width="' + gw + '" height="' + labelH + '"/></clipPath>';
        svg += '<text x="' + (gx + 4) + '" y="' + (gy + 11) +
            '" clip-path="url(#' + glClip + ')" class="treemap-group-label" pointer-events="none">' +
            escapeHtml(group.name) + '</text>';

        var innerRect = { x: gx + 1, y: gy + labelH, w: gw - 2, h: gh - labelH - 1 };
        if (innerRect.w > 0 && innerRect.h > 0) {
            var leafRects = squarify(group.items.slice(), innerRect);
            for (var lr = 0; lr < leafRects.length; lr++) {
                var leaf = leafRects[lr];
                var alpha = critAlpha(leaf.criticality);
                var color = statusColor(leaf.status);
                _scenarioRects[leaf.id] = { x: leaf.x, y: leaf.y, w: leaf.w, h: leaf.h };

                svg += '<rect class="treemap-leaf" data-type="scenario" data-id="' + escapeHtml(leaf.id) + '"' +
                    ' data-tooltip="' + escapeHtml(leaf.id) + ' \u2014 ' + escapeHtml(leaf.name) + (leaf.wip ? ' (WIP)' : '') + '"' +
                    ' data-group="' + escapeHtml(leaf.specName) + '" data-label="' + escapeHtml(leaf.id) + '"' +
                    ' x="' + leaf.x + '" y="' + leaf.y + '" width="' + leaf.w + '" height="' + leaf.h + '"' +
                    ' fill="' + color + '" opacity="' + alpha + '"' +
                    ' stroke="#0a0a0f" stroke-width="1" rx="2"/>';

                if (leaf.wip) {
                    svg += '<rect x="' + leaf.x + '" y="' + leaf.y + '" width="' + leaf.w + '" height="' + leaf.h + '"' +
                        ' fill="url(#wip-stripes)" opacity="0.4" pointer-events="none" rx="2"/>';
                }

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
            '" fill="#1a1a2e" stroke="#2a2a3e" stroke-width="0.5"' +
            ' data-tooltip="' + escapeHtml(group.name) + ' (' + group.items.length + ' files)"' +
            ' data-group-name="' + escapeHtml(group.name) + '"/>';
        var glClip2 = 'gc' + (_clipIdCounter++);
        svg += '<clipPath id="' + glClip2 + '"><rect x="' + gx + '" y="' + gy +
            '" width="' + gw + '" height="' + labelH + '"/></clipPath>';
        svg += '<text x="' + (gx + 4) + '" y="' + (gy + 11) +
            '" clip-path="url(#' + glClip2 + ')" class="treemap-group-label" pointer-events="none">' +
            escapeHtml(group.name) + '</text>';

        var innerRect = { x: gx + 1, y: gy + labelH, w: gw - 2, h: gh - labelH - 1 };
        if (innerRect.w > 0 && innerRect.h > 0) {
            var leafRects = squarify(group.items.slice(), innerRect);
            for (var lr = 0; lr < leafRects.length; lr++) {
                var leaf = leafRects[lr];
                _fileRects[leaf.id] = { x: leaf.x, y: leaf.y, w: leaf.w, h: leaf.h };

                var color = fileStatusColor(leaf.scenarios, _scenarioStatusMap);

                svg += '<rect class="treemap-leaf" data-type="file" data-id="' + escapeHtml(leaf.id) + '"' +
                    ' data-tooltip="' + escapeHtml(leaf.id) + ' (' + leaf.scenarioCount + ' scenarios)"' +
                    ' data-group="' + escapeHtml(leaf.crateName) + '" data-label="' + escapeHtml(leaf.name) + '"' +
                    ' x="' + leaf.x + '" y="' + leaf.y + '" width="' + leaf.w + '" height="' + leaf.h + '"' +
                    ' fill="' + color + '" opacity="0.75"' +
                    ' stroke="#0a0a0f" stroke-width="1" rx="2"/>';

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
    var leaves = document.querySelectorAll('.treemap-leaf');
    for (var i = 0; i < leaves.length; i++) {
        leaves[i].style.opacity = '0.15';
    }

    var highlighted = [];
    var linkedIds = [];
    if (type === 'scenario') {
        var el = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(id) + '"]');
        if (el) { el.style.opacity = '1'; highlighted.push(el); }
        linkedIds = (_scenarioLookup && _scenarioLookup[id]) || [];
        for (var j = 0; j < linkedIds.length; j++) {
            var fileEl = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(linkedIds[j]) + '"]');
            if (fileEl) { fileEl.style.opacity = '1'; highlighted.push(fileEl); }
        }
    } else if (type === 'file') {
        var fileLeaf = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(id) + '"]');
        if (fileLeaf) { fileLeaf.style.opacity = '1'; highlighted.push(fileLeaf); }
        linkedIds = (_codeLookup && _codeLookup[id]) || [];
        for (var k = 0; k < linkedIds.length; k++) {
            var scEl = document.querySelector('.treemap-leaf[data-id="' + CSS.escape(linkedIds[k]) + '"]');
            if (scEl) { scEl.style.opacity = '1'; highlighted.push(scEl); }
        }
    }

    showActiveLabels(highlighted);
    drawBridgeCurves(type, id, linkedIds);
}

function clearHighlight() {
    var leaves = document.querySelectorAll('.treemap-leaf');
    for (var i = 0; i < leaves.length; i++) {
        leaves[i].style.opacity = '';
    }
    clearActiveLabels();
    var bridgeSvg = document.getElementById('bridge-svg');
    if (bridgeSvg) bridgeSvg.innerHTML = '';
}

function showActiveLabels(highlightedEls) {
    clearActiveLabels();
    var groupsSeen = Object.create(null);

    for (var i = 0; i < highlightedEls.length; i++) {
        var el = highlightedEls[i];
        var svg = el.closest('svg');
        if (!svg) continue;

        var overlay = svg.querySelector('.active-label-overlay');
        if (!overlay) {
            overlay = document.createElementNS('http://www.w3.org/2000/svg', 'g');
            overlay.setAttribute('class', 'active-label-overlay');
            svg.appendChild(overlay);
        }

        // Show full leaf label
        var lx = parseFloat(el.getAttribute('x'));
        var ly = parseFloat(el.getAttribute('y'));
        var lw = parseFloat(el.getAttribute('width'));
        var lh = parseFloat(el.getAttribute('height'));
        var label = el.dataset.label || el.dataset.id || '';
        if (label && lh >= 10) {
            var txt = document.createElementNS('http://www.w3.org/2000/svg', 'text');
            txt.setAttribute('x', lx + lw / 2);
            txt.setAttribute('y', ly + lh / 2 + 3);
            txt.setAttribute('text-anchor', 'middle');
            txt.setAttribute('class', 'active-label-text');
            txt.textContent = label;
            overlay.appendChild(txt);
        }

        // Show full group header label (once per group per SVG)
        var groupName = el.dataset.group;
        var svgId = svg.closest('.treemap-container') ? svg.closest('.treemap-container').id : '';
        var groupKey = svgId + ':' + groupName;
        if (groupName && !groupsSeen[groupKey]) {
            groupsSeen[groupKey] = true;
            var header = svg.querySelector('rect[data-group-name="' + CSS.escape(groupName) + '"]');
            if (header) {
                var hx = parseFloat(header.getAttribute('x'));
                var hy = parseFloat(header.getAttribute('y'));
                var htxt = document.createElementNS('http://www.w3.org/2000/svg', 'text');
                htxt.setAttribute('x', hx + 4);
                htxt.setAttribute('y', hy + 11);
                htxt.setAttribute('class', 'active-group-text');
                htxt.textContent = groupName;
                overlay.appendChild(htxt);
            }
        }
    }
}

function clearActiveLabels() {
    var overlays = document.querySelectorAll('.active-label-overlay');
    for (var i = 0; i < overlays.length; i++) {
        overlays[i].parentNode.removeChild(overlays[i]);
    }
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
        if (scenario.wip && scenario.issue) {
            var issueNum = scenario.issue.split('/').pop();
            html += '<div class="trace-detail-section"><strong>Issue:</strong>' +
                '<div class="trace-detail-item"><a href="' + escapeHtml(scenario.issue) +
                '" target="_blank" rel="noopener" style="color:#a855f7">#' +
                escapeHtml(issueNum) + '</a> (WIP)</div></div>';
        }
        html += '</div>';
        detailEl.innerHTML = html;
    } else if (type === 'file') {
        var linkedScenarios = (_codeLookup && _codeLookup[id]) || [];
        var html2 = '<div class="trace-detail-card">';
        html2 += '<div class="trace-detail-header" style="font-family:monospace">' + escapeHtml(id) + '</div>';
        html2 += '<div class="trace-detail-meta">' + linkedScenarios.length + ' linked scenarios</div>';
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
    if (!scenarioIds || scenarioIds.length === 0) return '#6b7280';
    var covered = 0;
    for (var i = 0; i < scenarioIds.length; i++) {
        var st = statusMap && statusMap[scenarioIds[i]];
        if (st === 'covered' || st === 'partial') covered++;
    }
    var ratio = covered / scenarioIds.length;
    if (ratio >= 0.8) return '#22c55e';
    if (ratio >= 0.4) return '#eab308';
    return '#ef4444';
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
