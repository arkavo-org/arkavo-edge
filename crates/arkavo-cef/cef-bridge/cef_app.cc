#include "cef_app.h"
// #include "dom_executor.h" - Temporarily disabled due to CEF API compatibility
#include <iostream>

ArkavoRenderProcessHandler::ArkavoRenderProcessHandler(const std::string& socket_path)
    : socket_path_(socket_path) {
}

void ArkavoRenderProcessHandler::OnContextCreated(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefV8Context> context) {

    std::cout << "Arkavo CEF context created (V8 disabled)" << std::endl;

    // DOMExecutor::GetInstance()->Initialize(frame, socket_path_); - Temporarily disabled
    std::cout << "Note: DOM manipulation temporarily disabled pending CEF API update" << std::endl;
}

void ArkavoRenderProcessHandler::OnContextReleased(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefV8Context> context) {

    std::cout << "Arkavo CEF context released" << std::endl;
}

ArkavoApp::ArkavoApp(const std::string& socket_path) {
    render_process_handler_ = new ArkavoRenderProcessHandler(socket_path);
}
