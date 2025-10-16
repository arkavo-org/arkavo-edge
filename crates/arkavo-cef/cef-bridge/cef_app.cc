#include "cef_app.h"
#include "dom_executor.h"
#include <iostream>

ArkavoRenderProcessHandler::ArkavoRenderProcessHandler(const std::string& socket_path)
    : socket_path_(socket_path) {
}

void ArkavoRenderProcessHandler::OnContextCreated(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefV8Context> context) {

    std::cout << "Arkavo CEF context created (single-process: DOMExecutor already initialized in OnAfterCreated)" << std::endl;
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

    // Enable single-process mode (avoids multi-process IPC issues)
    command_line->AppendSwitch("single-process");

    // Single-process mode: software rendering via SwiftShader
    command_line->AppendSwitchWithValue("use-angle", "swiftshader");
    command_line->AppendSwitchWithValue("use-gl", "swiftshader");

    // Disable GPU-accelerated features (software rendering only)
    command_line->AppendSwitch("disable-gpu-compositing");

    // Reduce compositor/gpu paths for software rendering
    command_line->AppendSwitchWithValue("disable-features",
        "VizDisplayCompositor,UseSkiaRenderer,CanvasOopRasterization,"
        "Accelerated2dCanvas,ThreadedDisplayCompositor");

    // OSR-specific: enable frame scheduling for windowless rendering
    command_line->AppendSwitch("enable-begin-frame-scheduling");

    // Disable keychain/password manager to prevent login keychain prompts
    command_line->AppendSwitch("use-mock-keychain");
    command_line->AppendSwitch("password-store=basic");

    // Disable features not needed for UI generation
    command_line->AppendSwitch("disable-sync");

    std::cout << "Single-process mode: software rendering configured"
              << std::endl;
}

