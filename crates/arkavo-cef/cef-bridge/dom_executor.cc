#include "dom_executor.h"
#include "include/cef_task.h"
#include <chrono>
#include <iostream>
#include <sstream>

DOMExecutor* DOMExecutor::GetInstance() {
    static DOMExecutor instance;
    return &instance;
}

void DOMExecutor::Initialize(CefRefPtr<CefFrame> frame, const std::string& socket_path) {
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
}

void DOMExecutor::ProcessCommand(const DOMCommand& cmd) {
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
    for (char c : str) {
        switch (c) {
            case '"':  escaped << "\\\""; break;
            case '\\': escaped << "\\\\"; break;
            case '\n': escaped << "\\n"; break;
            case '\r': escaped << "\\r"; break;
            case '\t': escaped << "\\t"; break;
            default:   escaped << c; break;
        }
    }
    return escaped.str();
}

void DOMExecutor::ExecuteReplaceInnerHTML(uint32_t id, const std::string& selector, const std::string& html) {
    if (!frame_) {
        DOMFeedback feedback = {id, 2, 0, "Frame not available"};
        SendFeedback(feedback);
        return;
    }

    std::string escaped_html = EscapeJavaScript(html);
    std::string escaped_selector = EscapeJavaScript(selector);

    std::ostringstream js;
    js << "(function() { "
       << "  try { "
       << "    var el = document.querySelector(\"" << escaped_selector << "\"); "
       << "    if (!el) throw new Error('Element not found: " << escaped_selector << "'); "
       << "    el.innerHTML = \"" << escaped_html << "\"; "
       << "    return 'OK'; "
       << "  } catch(e) { "
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetAttribute(uint32_t id, const std::string& selector,
                                       const std::string& attr, const std::string& value) {
    if (!frame_) {
        DOMFeedback feedback = {id, 2, 0, "Frame not available"};
        SendFeedback(feedback);
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
        DOMFeedback feedback = {id, 2, 0, "Frame not available"};
        SendFeedback(feedback);
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
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetTextContent(uint32_t id, const std::string& selector, const std::string& text) {
    if (!frame_) {
        DOMFeedback feedback = {id, 2, 0, "Frame not available"};
        SendFeedback(feedback);
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
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteRemoveNode(uint32_t id, const std::string& selector) {
    if (!frame_) {
        DOMFeedback feedback = {id, 2, 0, "Frame not available"};
        SendFeedback(feedback);
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
       << "    throw e; "
       << "  } "
       << "})();";

    frame_->ExecuteJavaScript(js.str(), frame_->GetURL(), 0);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::SendFeedback(const DOMFeedback& feedback) {
    if (uds_client_) {
        uds_client_->SendFeedback(feedback);
    }
}
