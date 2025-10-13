#include "cef_app.h"
#include "include/cef_app.h"
#include "include/wrapper/cef_helpers.h"
#include <iostream>
#include <string>

int main(int argc, char* argv[]) {
    std::string socket_path = "/tmp/arkavo_dom.sock";

    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--socket" && i + 1 < argc) {
            socket_path = argv[++i];
        }
    }

    std::cout << "Arkavo CEF Renderer starting..." << std::endl;
    std::cout << "Socket path: " << socket_path << std::endl;

    CefMainArgs main_args(argc, argv);

    CefRefPtr<ArkavoApp> app = new ArkavoApp(socket_path);

    CefSettings settings;
    settings.no_sandbox = true;
    settings.windowless_rendering_enabled = true;
    settings.javascript_flags = CefString("--disable-javascript");
    CefString(&settings.log_file).FromASCII("/tmp/arkavo_cef.log");
    settings.log_severity = LOGSEVERITY_WARNING;

    std::cout << "Initializing CEF..." << std::endl;

    int exit_code = CefExecuteProcess(main_args, app.get(), nullptr);
    if (exit_code >= 0) {
        return exit_code;
    }

    bool initialized = CefInitialize(main_args, settings, app.get(), nullptr);
    if (!initialized) {
        std::cerr << "Failed to initialize CEF" << std::endl;
        return 1;
    }

    std::cout << "CEF initialized successfully" << std::endl;

    CefRunMessageLoop();

    CefShutdown();

    std::cout << "Arkavo CEF Renderer shutdown" << std::endl;

    return 0;
}
