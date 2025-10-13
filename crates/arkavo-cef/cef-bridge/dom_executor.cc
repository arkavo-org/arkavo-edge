#include "dom_executor.h"
#include "include/cef_task.h"
#include "include/cef_v8.h"
#include <chrono>
#include <iostream>

DOMExecutor* DOMExecutor::GetInstance() {
    static DOMExecutor instance;
    return &instance;
}

void DOMExecutor::Initialize(CefRefPtr<CefFrame> frame, const std::string& socket_path) {
    frame_ = frame;
    uds_client_ = std::make_unique<UdsClient>(socket_path);

    if (!uds_client_->Connect()) {
        std::cerr << "Failed to connect to UDS at " << socket_path << std::endl;
        return;
    }

    std::cout << "DOMExecutor initialized with socket: " << socket_path << std::endl;

    uds_client_->StartListening([this](const DOMCommand& cmd) {
        ProcessCommand(cmd);
    });
}

CefRefPtr<CefDOMNode> DOMExecutor::FindNode(const std::string& selector) {
    if (!frame_) {
        return nullptr;
    }

    CefRefPtr<CefDOMDocument> document = frame_->GetDOM();
    if (!document) {
        return nullptr;
    }

    return document->GetElementById(selector);
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

void DOMExecutor::ExecuteReplaceInnerHTML(uint32_t id, const std::string& selector, const std::string& html) {
    CefRefPtr<CefDOMNode> node = FindNode(selector);

    if (!node) {
        DOMFeedback feedback = {id, 2, 0, "Element not found: " + selector};
        SendFeedback(feedback);
        return;
    }

    node->SetValue(html);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetAttribute(uint32_t id, const std::string& selector, const std::string& attr, const std::string& value) {
    CefRefPtr<CefDOMNode> node = FindNode(selector);

    if (!node) {
        DOMFeedback feedback = {id, 2, 0, "Element not found: " + selector};
        SendFeedback(feedback);
        return;
    }

    node->SetElementAttribute(attr, value);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetStyle(uint32_t id, const std::string& selector, const std::string& property, const std::string& value) {
    CefRefPtr<CefDOMNode> node = FindNode(selector);

    if (!node) {
        DOMFeedback feedback = {id, 2, 0, "Element not found: " + selector};
        SendFeedback(feedback);
        return;
    }

    std::string style_attr = node->GetElementAttribute("style");
    style_attr += property + ": " + value + ";";
    node->SetElementAttribute("style", style_attr);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteSetTextContent(uint32_t id, const std::string& selector, const std::string& text) {
    CefRefPtr<CefDOMNode> node = FindNode(selector);

    if (!node) {
        DOMFeedback feedback = {id, 2, 0, "Element not found: " + selector};
        SendFeedback(feedback);
        return;
    }

    node->SetValue(text);

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::ExecuteRemoveNode(uint32_t id, const std::string& selector) {
    CefRefPtr<CefDOMNode> node = FindNode(selector);

    if (!node) {
        DOMFeedback feedback = {id, 2, 0, "Element not found: " + selector};
        SendFeedback(feedback);
        return;
    }

    CefRefPtr<CefDOMNode> parent = node->GetParent();
    if (parent) {
        parent->RemoveChild(node);
    }

    DOMFeedback feedback = {id, 0, 0, "OK"};
    SendFeedback(feedback);
}

void DOMExecutor::SendFeedback(const DOMFeedback& feedback) {
    if (uds_client_) {
        uds_client_->SendFeedback(feedback);
    }
}
