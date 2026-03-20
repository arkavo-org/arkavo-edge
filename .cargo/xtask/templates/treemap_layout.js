"use strict";

// Squarified treemap layout (Bruls-Huizing-van Wijk)
// Pure functions, no DOM dependency

function squarify(items, rect) {
    if (!items || items.length === 0) return [];

    var total = 0;
    for (var i = 0; i < items.length; i++) total += items[i].value;
    if (total === 0) return items;

    var area = rect.w * rect.h;
    var sorted = items.slice().sort(function(a, b) { return b.value - a.value; });

    var rects = [];
    layoutStrip(sorted, 0, sorted.length, rect, total, rects);
    return rects;
}

function layoutStrip(items, start, end, rect, totalValue, out) {
    if (start >= end) return;
    if (end - start === 1) {
        items[start].x = rect.x;
        items[start].y = rect.y;
        items[start].w = rect.w;
        items[start].h = rect.h;
        out.push(items[start]);
        return;
    }

    var area = rect.w * rect.h;
    var sum = 0;
    for (var i = start; i < end; i++) sum += items[i].value;

    var bestIdx = start + 1;
    var bestAspect = Infinity;
    var runSum = items[start].value;

    for (var j = start + 1; j < end; j++) {
        var frac = runSum / sum;
        var aspect = worstAspect(items, start, j, rect, frac, sum, area);
        runSum += items[j].value;
        var nextFrac = runSum / sum;
        var nextAspect = worstAspect(items, start, j + 1, rect, nextFrac, sum, area);
        if (nextAspect > aspect) {
            bestIdx = j;
            bestAspect = aspect;
            break;
        }
        if (j === end - 1) {
            bestIdx = end;
        }
    }

    var stripSum = 0;
    for (var k = start; k < bestIdx; k++) stripSum += items[k].value;
    var stripFrac = stripSum / sum;

    var r1, r2;
    if (rect.w >= rect.h) {
        var splitW = rect.w * stripFrac;
        r1 = { x: rect.x, y: rect.y, w: splitW, h: rect.h };
        r2 = { x: rect.x + splitW, y: rect.y, w: rect.w - splitW, h: rect.h };
    } else {
        var splitH = rect.h * stripFrac;
        r1 = { x: rect.x, y: rect.y, w: rect.w, h: splitH };
        r2 = { x: rect.x, y: rect.y + splitH, w: rect.w, h: rect.h - splitH };
    }

    layoutSlice(items, start, bestIdx, r1, stripSum, out);
    layoutStrip(items, bestIdx, end, r2, sum - stripSum, out);
}

function layoutSlice(items, start, end, rect, total, out) {
    var offset = 0;
    for (var i = start; i < end; i++) {
        var frac = items[i].value / total;
        if (rect.w >= rect.h) {
            items[i].x = rect.x;
            items[i].y = rect.y + offset;
            items[i].w = rect.w;
            items[i].h = rect.h * frac;
            offset += items[i].h;
        } else {
            items[i].x = rect.x + offset;
            items[i].y = rect.y;
            items[i].w = rect.w * frac;
            items[i].h = rect.h;
            offset += items[i].w;
        }
        out.push(items[i]);
    }
}

function worstAspect(items, start, end, rect, frac, total, area) {
    var worst = 0;
    var stripArea = area * frac;
    var side = (rect.w >= rect.h) ? rect.h : rect.w;
    if (side === 0) return Infinity;
    var stripLen = stripArea / side;
    if (stripLen === 0) return Infinity;

    var subTotal = 0;
    for (var i = start; i < end; i++) subTotal += items[i].value;

    for (var j = start; j < end; j++) {
        var itemFrac = items[j].value / subTotal;
        var itemSide = side * itemFrac;
        if (itemSide === 0) continue;
        var aspect = Math.max(stripLen / itemSide, itemSide / stripLen);
        if (aspect > worst) worst = aspect;
    }
    return worst;
}

function statusColor(status) {
    switch (status) {
        case 'covered': return '#22c55e';
        case 'partial': return '#eab308';
        case 'missing': return '#ef4444';
        default: return '#6b7280';
    }
}

function critAlpha(crit) {
    switch (crit) {
        case 'critical': return 1.0;
        case 'high': return 0.85;
        case 'medium': return 0.7;
        case 'low': return 0.55;
        default: return 0.7;
    }
}

function buildCodeLookup(links) {
    var lookup = Object.create(null);
    for (var i = 0; i < links.length; i++) {
        var file = links[i].file;
        if (!lookup[file]) lookup[file] = [];
        if (lookup[file].indexOf(links[i].scenario) === -1) {
            lookup[file].push(links[i].scenario);
        }
    }
    return lookup;
}

function buildScenarioLookup(links) {
    var lookup = Object.create(null);
    for (var i = 0; i < links.length; i++) {
        var sid = links[i].scenario;
        if (!lookup[sid]) lookup[sid] = [];
        if (lookup[sid].indexOf(links[i].file) === -1) {
            lookup[sid].push(links[i].file);
        }
    }
    return lookup;
}
