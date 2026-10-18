"use strict";

(function() {
    var bubble = document.createElement('div');
    bubble.id = 'information-help';
    bubble.className = 'information-help';
    bubble.setAttribute('role', 'tooltip');
    bubble.setAttribute('popover', 'manual');
    bubble.hidden = true;
    document.body.appendChild(bubble);
    var anchor = null;
    var pinned = false;
    var timer;

    function close() {
        clearTimeout(timer);
        if (anchor) anchor.removeAttribute('aria-describedby');
        if (typeof bubble.hidePopover === 'function') bubble.hidePopover();
        bubble.hidden = true;
        anchor = null;
        pinned = false;
    }

    function position() {
        if (!anchor) return;
        var box = anchor.getBoundingClientRect();
        var width = bubble.offsetWidth;
        bubble.style.left = Math.max(8, Math.min(box.left, innerWidth - width - 8)) + 'px';
        bubble.style.top = Math.max(8, Math.min(box.bottom + 8, innerHeight - bubble.offsetHeight - 8)) + 'px';
    }

    function open(button) {
        clearTimeout(timer);
        if (pinned && anchor !== button) return;
        var term = UI_TERMS[button.dataset.help];
        if (!term) return;
        // Help inside a modal must share its subtree to remain accessible.
        var host = button.closest('dialog') || document.body;
        if (bubble.parentElement !== host) {
            if (typeof bubble.hidePopover === 'function') bubble.hidePopover();
            host.appendChild(bubble);
        }
        if (anchor) anchor.removeAttribute('aria-describedby');
        anchor = button;
        bubble.textContent = term[2];
        bubble.hidden = false;
        if (typeof bubble.showPopover === 'function') bubble.showPopover();
        button.setAttribute('aria-describedby', bubble.id);
        position();
    }

    function deferClose() {
        clearTimeout(timer);
        timer = setTimeout(function() {
            if (!pinned && document.activeElement !== anchor && !bubble.matches(':hover')) close();
        }, 160);
    }

    document.addEventListener('pointerover', function(e) {
        var button = e.target.closest('[data-help]');
        if (button) open(button);
        if (bubble.contains(e.target)) clearTimeout(timer);
    });
    document.addEventListener('pointerout', function(e) {
        if (e.target.closest('[data-help]') || bubble.contains(e.target)) deferClose();
    });
    document.addEventListener('focusin', function(e) {
        if (e.target.matches('[data-help]')) open(e.target);
    });
    document.addEventListener('focusout', deferClose);
    document.addEventListener('click', function(e) {
        var button = e.target.closest('[data-help]');
        if (button) {
            if (anchor === button && pinned) close();
            else { close(); open(button); pinned = true; }
        } else if (!bubble.contains(e.target)) close();
    });
    document.addEventListener('keydown', function(e) {
        if (e.key === 'Escape' && anchor) {
            e.preventDefault();
            close();
        }
    });
    window.addEventListener('resize', position);
    document.addEventListener('scroll', position, true);
})();
