#include "dom_executor.h"
#include "include/cef_task.h"
#include <chrono>
#include <iostream>
#include <sstream>
#include <regex>
#include <algorithm>
#include <iomanip>

DOMExecutor* DOMExecutor::GetInstance() {
    static DOMExecutor instance;
    return &instance;
}

void DOMExecutor::Initialize(CefRefPtr<CefFrame> frame, const std::string& socket_path) {
    if (initialized_) {
        std::cout << "DOMExecutor already initialized, skipping duplicate initialization" << std::endl;
        return;
    }

    frame_ = frame;
    uds_client_ = std::make_unique<UdsClient>(socket_path);

    if (!uds_client_->Bind()) {
        std::cerr << "Failed to bind UDS server at " << socket_path << std::endl;
        return;
    }

    std::cout << "DOMExecutor initialized with socket: " << socket_path << std::endl;

    uds_client_->StartListening([this](const DOMCommand& cmd) {
        ProcessCommand(cmd);
    });

    RegisterEventBridge();

    initialized_ = true;
}

void DOMExecutor::Shutdown() {
    if (!initialized_) {
        return;
    }

    std::cout << "DOMExecutor shutting down..." << std::endl;

    // Disconnect UDS client first to stop listening threads
    if (uds_client_) {
        uds_client_->Disconnect();
        uds_client_.reset();
    }

    // Clear frame reference
    frame_ = nullptr;
    initialized_ = false;

    std::cout << "DOMExecutor shutdown complete" << std::endl;
}

void DOMExecutor::RegisterEventBridge() {
    if (!frame_) {
        std::cerr << "Cannot register event bridge: frame not available" << std::endl;
        return;
    }

    std::ostringstream js;
    js << "(function() {"
       << "  window.ArkavoEventBridge = function(event) {"
       << "    if (!event || typeof event !== 'object') {"
       << "      console.error('ArkavoEventBridge: Invalid event object');"
       << "      return;"
       << "    }"
       << "    if (typeof window.arkavoPushEvent === 'function') {"
       << "      window.arkavoPushEvent(event);"
       << "    } else {"
       << "      console.warn('arkavoPushEvent not available, queueing event');"
       << "      window.__arkavoEventQueue = window.__arkavoEventQueue || [];"
       << "      window.__arkavoEventQueue.push(event);"
       << "    }"
       << "  };"
       << "  window.__arkavoEventQueue = [];"
       << "  console.log('ArkavoEventBridge registered (push mode)');"
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);
    std::cout << "ArkavoEventBridge function registered in window context" << std::endl;
}

void DOMExecutor::PollEventQueue() {
    if (!frame_ || !initialized_) {
        return;
    }

    std::ostringstream js;
    js << "(function() {"
       << "  var events = window.__arkavoGetEvents ? window.__arkavoGetEvents() : null;"
       << "  if (events && events.length > 0) {"
       << "    console.log('Polled ' + events.length + ' events');"
       << "  }"
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);
}

void DOMExecutor::HandleDOMEvent(const std::string& event_type, const std::string& selector,
                                  const std::string& target_id, const std::string& value,
                                  const std::string& data) {
    DOMEvent event;
    event.event_type = event_type;
    event.selector = selector;
    event.target_id = target_id;
    event.value = value;
    event.data = data;

    SendEvent(event);
    std::cout << "DOM event sent: " << event_type << " on " << selector << std::endl;
}

void DOMExecutor::ProcessCommand(const DOMCommand& cmd) {
    std::cout << "[DEBUG C++] ProcessCommand called: op=" << static_cast<int>(cmd.op)
              << ", id=" << cmd.id
              << ", selector='" << cmd.selector << "'" << std::endl;

    auto start = std::chrono::high_resolution_clock::now();

    switch (cmd.op) {
        case DOMOp::ReplaceInnerHTML:
            ExecuteReplaceInnerHTML(cmd.id, cmd.selector, cmd.payload);
            break;
        case DOMOp::SetAttribute:
            ExecuteSetAttribute(cmd.id, cmd.selector, cmd.property, cmd.payload);
            break;
        case DOMOp::SetStyle:
            ExecuteSetStyle(cmd.id, cmd.selector, cmd.property, cmd.payload);
            break;
        case DOMOp::SetTextContent:
            ExecuteSetTextContent(cmd.id, cmd.selector, cmd.payload);
            break;
        case DOMOp::RemoveNode:
            ExecuteRemoveNode(cmd.id, cmd.selector);
            break;
        case DOMOp::AddEventListener:
            ExecuteAddEventListener(cmd.id, cmd.selector, cmd.payload);
            break;
        default:
            DOMFeedback feedback = {cmd.id, 1, 0, "Unknown operation"};
            SendFeedback(feedback);
            break;
    }

    auto end = std::chrono::high_resolution_clock::now();
    auto duration = std::chrono::duration_cast<std::chrono::nanoseconds>(end - start);

    std::cout << "Command " << cmd.id << " executed in " << duration.count() << "ns" << std::endl;
}

std::string DOMExecutor::EscapeJavaScript(const std::string& str) {
    std::ostringstream escaped;
    for (unsigned char c : str) {
        switch (c) {
            case '"':  escaped << "\\\""; break;
            case '\'': escaped << "\\'"; break;
            case '\\': escaped << "\\\\"; break;
            case '\n': escaped << "\\n"; break;
            case '\r': escaped << "\\r"; break;
            case '\t': escaped << "\\t"; break;
            case '\b': escaped << "\\b"; break;
            case '\f': escaped << "\\f"; break;
            case '`':  escaped << "\\`"; break;
            case '<':  escaped << "\\x3C"; break;
            case '>':  escaped << "\\x3E"; break;
            case '&':  escaped << "\\x26"; break;
            case '/':  escaped << "\\/"; break;
            default:
                if (c < 0x20 || c >= 0x7F) {
                    escaped << "\\x" << std::hex << std::setw(2) << std::setfill('0') << static_cast<int>(c);
                } else {
                    escaped << c;
                }
                break;
        }
    }
    return escaped.str();
}

bool DOMExecutor::ValidateSelector(const std::string& selector) {
    if (selector.empty() || selector.length() > MAX_SELECTOR_LENGTH) {
        return false;
    }

    static const std::regex valid_selector_regex(
        R"(^[#.]?[a-zA-Z0-9_-]+(\s*[>+~]\s*[#.]?[a-zA-Z0-9_-]+)*$)"
    );

    return std::regex_match(selector, valid_selector_regex);
}

bool DOMExecutor::ValidatePayloadSize(const std::string& payload, size_t max_size) {
    return payload.size() <= max_size;
}

std::string DOMExecutor::SanitizeHtml(const std::string& html) {
    std::string sanitized = html;

    static const std::vector<std::string> dangerous_patterns = {
        "<script", "</script>",
        "javascript:", "onerror=", "onload=",
        "onclick=", "onmouseover=", "onfocus="
    };

    for (const auto& pattern : dangerous_patterns) {
        size_t pos = 0;
        std::string pattern_lower = pattern;
        std::transform(pattern_lower.begin(), pattern_lower.end(), pattern_lower.begin(), ::tolower);

        while ((pos = sanitized.find(pattern, pos)) != std::string::npos) {
            std::string check = sanitized.substr(pos, pattern.length());
            std::transform(check.begin(), check.end(), check.begin(), ::tolower);

            if (check == pattern_lower) {
                sanitized.replace(pos, pattern.length(), "");
            } else {
                pos++;
            }
        }
    }

    return sanitized;
}

void DOMExecutor::SendErrorFeedback(uint32_t id, const std::string& message) {
    DOMFeedback feedback = {id, 1, 0, message};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteReplaceInnerHTML(uint32_t id, const std::string& selector, const std::string& html) {
    if (!frame_) {
        SendErrorFeedback(id, "Frame not available");
        return;
    }

    if (!ValidateSelector(selector)) {
        SendErrorFeedback(id, "Invalid selector");
        return;
    }

    if (!ValidatePayloadSize(html, MAX_HTML_PAYLOAD_SIZE)) {
        SendErrorFeedback(id, "HTML payload too large");
        return;
    }

    std::string sanitized_html = SanitizeHtml(html);
    std::string escaped_html = EscapeJavaScript(sanitized_html);
    std::string escaped_selector = EscapeJavaScript(selector);

    std::ostringstream js;
    js << "(function() { "
       << "  try { "
       << "    var el = document.querySelector(\"" << escaped_selector << "\"); "
       << "    if (!el) throw new Error('Element not found: " << escaped_selector << "'); "
       << "    el.innerHTML = \"" << escaped_html << "\"; "
       << "    return 'OK'; "
       << "  } catch(e) { "
       << "    if (typeof window.ArkavoEventBridge === 'function') { "
       << "      window.ArkavoEventBridge({"
       << "        event_type: 'js_error', "
       << "        selector: '" << escaped_selector << "', "
       << "        target_id: 'dom_executor', "
       << "        value: 'ReplaceInnerHTML failed', "
       << "        data: JSON.stringify({error: e.message, stack: e.stack, command_id: " << id << "})"
       << "      });"
       << "    }"
       << "    console.error('[CEF DOM] ReplaceInnerHTML error:', e); "
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    // Re-attach prompt bar event listeners after HTML replacement
    std::ostringstream setup_js;
    setup_js << "(function() {"
             << "  var input = document.getElementById('prompt-input');"
             << "  var button = document.getElementById('prompt-submit');"
             << "  if (!input || !button) return;"
             << "  "
             << "  function submitPrompt() {"
             << "    var value = input.value.trim();"
             << "    if (value.length === 0) return;"
             << "    if (typeof window.ArkavoEventBridge === 'function') {"
             << "      window.ArkavoEventBridge({"
             << "        event_type: 'submit',"
             << "        selector: '#prompt-input',"
             << "        target_id: 'prompt-input',"
             << "        value: value,"
             << "        data: '{}'"
             << "      });"
             << "      input.value = '';"
             << "    }"
             << "  }"
             << "  "
             << "  button.onclick = submitPrompt;"
             << "  input.onkeypress = function(e) { if (e.key === 'Enter') submitPrompt(); };"
             << "})();";

    frame_->ExecuteJavaScript(setup_js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetAttribute(uint32_t id, const std::string& selector,
                                       const std::string& attr, const std::string& value) {
    if (!frame_) {
        SendErrorFeedback(id, "Frame not available");
        return;
    }

    if (!ValidateSelector(selector)) {
        SendErrorFeedback(id, "Invalid selector");
        return;
    }

    if (!ValidatePayloadSize(value, MAX_HTML_PAYLOAD_SIZE)) {
        SendErrorFeedback(id, "Attribute value too large");
        return;
    }

    std::string escaped_selector = EscapeJavaScript(selector);
    std::string escaped_attr = EscapeJavaScript(attr);
    std::string escaped_value = EscapeJavaScript(value);

    std::ostringstream js;
    js << "(function() { "
       << "  try { "
       << "    var el = document.querySelector(\"" << escaped_selector << "\"); "
       << "    if (!el) throw new Error('Element not found: " << escaped_selector << "'); "
       << "    el.setAttribute(\"" << escaped_attr << "\", \"" << escaped_value << "\"); "
       << "    return 'OK'; "
       << "  } catch(e) { "
       << "    if (typeof window.ArkavoEventBridge === 'function') { "
       << "      window.ArkavoEventBridge({"
       << "        event_type: 'js_error', "
       << "        selector: '" << escaped_selector << "', "
       << "        target_id: 'dom_executor', "
       << "        value: 'SetAttribute failed', "
       << "        data: JSON.stringify({error: e.message, stack: e.stack, command_id: " << id << "})"
       << "      });"
       << "    }"
       << "    console.error('[CEF DOM] SetAttribute error:', e); "
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetStyle(uint32_t id, const std::string& selector,
                                   const std::string& property, const std::string& value) {
    if (!frame_) {
        SendErrorFeedback(id, "Frame not available");
        return;
    }

    if (!ValidateSelector(selector)) {
        SendErrorFeedback(id, "Invalid selector");
        return;
    }

    if (!ValidatePayloadSize(value, MAX_CSS_PAYLOAD_SIZE)) {
        SendErrorFeedback(id, "CSS value too large");
        return;
    }

    std::string escaped_selector = EscapeJavaScript(selector);
    std::string escaped_property = EscapeJavaScript(property);
    std::string escaped_value = EscapeJavaScript(value);

    std::ostringstream js;
    js << "(function() { "
       << "  try { "
       << "    var el = document.querySelector(\"" << escaped_selector << "\"); "
       << "    if (!el) throw new Error('Element not found: " << escaped_selector << "'); "
       << "    el.style[\"" << escaped_property << "\"] = \"" << escaped_value << "\"; "
       << "    return 'OK'; "
       << "  } catch(e) { "
       << "    if (typeof window.ArkavoEventBridge === 'function') { "
       << "      window.ArkavoEventBridge({"
       << "        event_type: 'js_error', "
       << "        selector: '" << escaped_selector << "', "
       << "        target_id: 'dom_executor', "
       << "        value: 'SetStyle failed', "
       << "        data: JSON.stringify({error: e.message, stack: e.stack, command_id: " << id << "})"
       << "      });"
       << "    }"
       << "    console.error('[CEF DOM] SetStyle error:', e); "
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetTextContent(uint32_t id, const std::string& selector, const std::string& text) {
    if (!frame_) {
        SendErrorFeedback(id, "Frame not available");
        return;
    }

    if (!ValidateSelector(selector)) {
        SendErrorFeedback(id, "Invalid selector");
        return;
    }

    if (!ValidatePayloadSize(text, MAX_HTML_PAYLOAD_SIZE)) {
        SendErrorFeedback(id, "Text payload too large");
        return;
    }

    std::string escaped_selector = EscapeJavaScript(selector);
    std::string escaped_text = EscapeJavaScript(text);

    std::ostringstream js;
    js << "(function() { "
       << "  try { "
       << "    var el = document.querySelector(\"" << escaped_selector << "\"); "
       << "    if (!el) throw new Error('Element not found: " << escaped_selector << "'); "
       << "    el.textContent = \"" << escaped_text << "\"; "
       << "    return 'OK'; "
       << "  } catch(e) { "
       << "    if (typeof window.ArkavoEventBridge === 'function') { "
       << "      window.ArkavoEventBridge({"
       << "        event_type: 'js_error', "
       << "        selector: '" << escaped_selector << "', "
       << "        target_id: 'dom_executor', "
       << "        value: 'SetTextContent failed', "
       << "        data: JSON.stringify({error: e.message, stack: e.stack, command_id: " << id << "})"
       << "      });"
       << "    }"
       << "    console.error('[CEF DOM] SetTextContent error:', e); "
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteRemoveNode(uint32_t id, const std::string& selector) {
    if (!frame_) {
        SendErrorFeedback(id, "Frame not available");
        return;
    }

    if (!ValidateSelector(selector)) {
        SendErrorFeedback(id, "Invalid selector");
        return;
    }

    std::string escaped_selector = EscapeJavaScript(selector);

    std::ostringstream js;
    js << "(function() { "
       << "  try { "
       << "    var el = document.querySelector(\"" << escaped_selector << "\"); "
       << "    if (!el) throw new Error('Element not found: " << escaped_selector << "'); "
       << "    el.parentNode.removeChild(el); "
       << "    return 'OK'; "
       << "  } catch(e) { "
       << "    if (typeof window.ArkavoEventBridge === 'function') { "
       << "      window.ArkavoEventBridge({"
       << "        event_type: 'js_error', "
       << "        selector: '" << escaped_selector << "', "
       << "        target_id: 'dom_executor', "
       << "        value: 'RemoveNode failed', "
       << "        data: JSON.stringify({error: e.message, stack: e.stack, command_id: " << id << "})"
       << "      });"
       << "    }"
       << "    console.error('[CEF DOM] RemoveNode error:', e); "
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteAddEventListener(uint32_t id, const std::string& selector, const std::string& event_type) {
    if (!frame_) {
        SendErrorFeedback(id, "Frame not available");
        return;
    }

    if (!ValidateSelector(selector)) {
        SendErrorFeedback(id, "Invalid selector");
        return;
    }

    std::string escaped_selector = EscapeJavaScript(selector);
    std::string escaped_event_type = EscapeJavaScript(event_type);

    std::ostringstream js;
    js << "(function() { "
       << "  try { "
       << "    var el = document.querySelector(\"" << escaped_selector << "\"); "
       << "    if (!el) throw new Error('Element not found: " << escaped_selector << "'); "
       << "    el.addEventListener(\"" << escaped_event_type << "\", function(e) { "
       << "      window.ArkavoEventBridge({"
       << "        event_type: \"" << escaped_event_type << "\", "
       << "        selector: \"" << escaped_selector << "\", "
       << "        target_id: e.target.id || '', "
       << "        value: e.target.value || '', "
       << "        data: JSON.stringify({clientX: e.clientX, clientY: e.clientY, key: e.key})"
       << "      });"
       << "    }); "
       << "    return 'OK'; "
       << "  } catch(e) { "
       << "    if (typeof window.ArkavoEventBridge === 'function') { "
       << "      window.ArkavoEventBridge({"
       << "        event_type: 'js_error', "
       << "        selector: '" << escaped_selector << "', "
       << "        target_id: 'dom_executor', "
       << "        value: 'AddEventListener failed', "
       << "        data: JSON.stringify({error: e.message, stack: e.stack, command_id: " << id << "})"
       << "      });"
       << "    }"
       << "    console.error('[CEF DOM] AddEventListener error:', e); "
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::SendEvent(const DOMEvent& event) {
    if (uds_client_) {
        bool success = uds_client_->SendEvent(event);
        if (!success) {
            std::cerr << "[ERROR] Failed to send event through UDS" << std::endl;
        }
    } else {
        std::cerr << "[ERROR] Cannot send event: uds_client_ is null" << std::endl;
    }
}

void DOMExecutor::SendFeedback(const DOMFeedback& feedback) {
    if (uds_client_) {
        uds_client_->SendFeedback(feedback);
    }
}

void DOMExecutor::SendError(const DOMError& error) {
    if (uds_client_) {
        uds_client_->SendError(error);
    }
}
