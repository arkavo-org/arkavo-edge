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

void ArkavoApp::OnBeforeCommandLineProcessing(
    const CefString& process_type,
    CefRefPtr<CefCommandLine> command_line) {

    // Completely disable GPU process
    command_line->AppendSwitch("disable-gpu");
    command_line->AppendSwitch("disable-gpu-compositing");
    command_line->AppendSwitch("disable-gpu-process-crash-limit");

    // Disable keychain/password manager to prevent login keychain prompts
    command_line->AppendSwitch("use-mock-keychain");
    command_line->AppendSwitch("password-store=basic");

    // Disable various features we don't need
    command_line->AppendSwitch("disable-features=RendererCodeIntegrity");
    command_line->AppendSwitch("disable-sync");

    std::cout << "Command line switches applied" << std::endl;
}
