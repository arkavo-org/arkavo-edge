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

    // Log GPU process command line for diagnostics
    if (process_type == "gpu-process") {
        std::cout << "GPU process CMD: "
                  << command_line->GetCommandLineString().ToString()
                  << std::endl;
    }

    // Use software GPU (SwiftShader) instead of disabling GPU entirely
    command_line->AppendSwitchWithValue("use-angle", "swiftshader");
    command_line->AppendSwitchWithValue("use-gl", "swiftshader");

    // Disable GPU-accelerated features, prefer software paths
    command_line->AppendSwitch("disable-gpu-compositing");

    // Reduce compositor/gpu paths that can trigger device init
    command_line->AppendSwitchWithValue("disable-features",
        "VizDisplayCompositor,UseSkiaRenderer,CanvasOopRasterization,"
        "Accelerated2dCanvas,ThreadedDisplayCompositor");

    // Stop GPU restart loop during bring-up
    command_line->AppendSwitchWithValue("gpu-process-crash-limit", "0");

    // OSR-specific: enable frame scheduling for windowless rendering
    command_line->AppendSwitch("enable-begin-frame-scheduling");

    // Disable GPU watchdog to see actual errors during bring-up
    command_line->AppendSwitch("disable-gpu-watchdog");

    // Disable keychain/password manager to prevent login keychain prompts
    command_line->AppendSwitch("use-mock-keychain");
    command_line->AppendSwitch("password-store=basic");

    // Disable various features we don't need
    command_line->AppendSwitch("disable-sync");

    std::cout << "Command line switches applied for process: "
              << (process_type.empty() ? "browser" : process_type.ToString())
              << std::endl;
}

void ArkavoApp::OnBeforeChildProcessLaunch(
    CefRefPtr<CefCommandLine> command_line) {

    // Propagate flags to child processes (GPU, renderer, etc.)
    command_line->AppendSwitch("disable-gpu");
    command_line->AppendSwitch("disable-gpu-compositing");

    // Force software pipeline in child process
    command_line->AppendSwitchWithValue("use-angle", "swiftshader");
    command_line->AppendSwitchWithValue("use-gl", "swiftshader");

    // Reduce compositor/gpu paths
    command_line->AppendSwitchWithValue("disable-features",
        "VizDisplayCompositor,UseSkiaRenderer,CanvasOopRasterization,"
        "Accelerated2dCanvas,ThreadedDisplayCompositor");

    // Stop restart spam
    command_line->AppendSwitchWithValue("gpu-process-crash-limit", "0");

    std::cout << "Child process launch flags applied" << std::endl;
}
