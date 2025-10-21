#include "cef_app.h"
#include "browser_client.h"
#include "include/cef_app.h"
#include "include/wrapper/cef_helpers.h"
#include <iostream>
#include <string>
#include <climits>

#ifdef __APPLE__
#include <mach-o/dyld.h>
#include "include/wrapper/cef_library_loader.h"
#include <Cocoa/Cocoa.h>
#endif

int main(int argc, char* argv[]) {
#ifdef __APPLE__
    // Initialize NSApplication for macOS
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

    // Load the CEF framework library at runtime instead of linking directly.
    // This is required on macOS.
    CefScopedLibraryLoader library_loader;
    if (!library_loader.LoadInMain()) {
        std::cerr << "Failed to load CEF framework!" << std::endl;
        return 1;
    }
#endif

    CefMainArgs main_args(argc, argv);

    // Parse socket path for browser process
    std::string socket_path = "/tmp/arkavo_dom.sock";
    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--socket" && i + 1 < argc) {
            socket_path = argv[++i];
        }
    }

    // Create app handler for both browser and subprocess
    CefRefPtr<ArkavoApp> app = new ArkavoApp(socket_path);

    // IMPORTANT: Handle subprocess execution first!
    // If this is a subprocess (--type=renderer, --type=gpu-process, etc.),
    // CefExecuteProcess will run the subprocess logic and return >= 0.
    // We must return immediately without touching CefSettings or CefInitialize.
    int exit_code = CefExecuteProcess(main_args, app.get(), nullptr);
    if (exit_code >= 0) {
        // This is a subprocess - exit immediately
        return exit_code;
    }

    // If we get here, we're the browser process
    std::cout << "Arkavo CEF Browser starting..." << std::endl;
    std::cout << "Socket path: " << socket_path << std::endl;

    // Get executable path for setting CEF paths
    char exe_path[PATH_MAX];
    std::string exe_dir;
#ifdef __APPLE__
    uint32_t size = sizeof(exe_path);
    if (_NSGetExecutablePath(exe_path, &size) == 0) {
        exe_dir = exe_path;
        size_t last_slash = exe_dir.find_last_of("/");
        if (last_slash != std::string::npos) {
            exe_dir = exe_dir.substr(0, last_slash);
        }
    }
#endif

    CefSettings settings;
    settings.no_sandbox = true;
#ifdef __APPLE__
    settings.windowless_rendering_enabled = true;
#else
    settings.windowless_rendering_enabled = false;
#endif

    // Enable verbose logging for debugging
    settings.log_severity = LOGSEVERITY_VERBOSE;
    CefString(&settings.log_file).FromASCII("/tmp/arkavo_cef.log");

#ifdef __APPLE__
    if (!exe_dir.empty()) {
        // Set all required paths explicitly
        std::string framework_path = exe_dir + "/../Frameworks/Chromium Embedded Framework.framework";
        std::string resources_path = framework_path + "/Resources";
        std::string locales_path = resources_path + "/Locales";

        std::cout << "Framework: " << framework_path << std::endl;
        std::cout << "Resources: " << resources_path << std::endl;
        std::cout << "Locales: " << locales_path << std::endl;
        std::cout << "Subprocess: " << exe_path << std::endl;

        CefString(&settings.framework_dir_path).FromASCII(framework_path.c_str());
        CefString(&settings.resources_dir_path).FromASCII(resources_path.c_str());
        CefString(&settings.locales_dir_path).FromASCII(locales_path.c_str());
        CefString(&settings.browser_subprocess_path).FromASCII(exe_path);
    }
#endif

    std::cout << "Initializing CEF browser process..." << std::endl;

    bool initialized = CefInitialize(main_args, settings, app.get(), nullptr);
    if (!initialized) {
        std::cerr << "Failed to initialize CEF" << std::endl;
        return 1;
    }

    std::cout << "CEF initialized successfully" << std::endl;

    CefRefPtr<ArkavoBrowserClient> client = new ArkavoBrowserClient(socket_path);

    CefWindowInfo window_info;
    CefBrowserSettings browser_settings;
    browser_settings.javascript = STATE_DISABLED;

#ifdef __APPLE__
    // On macOS, use windowless rendering for now (no visible window yet)
    window_info.SetAsWindowless(0);
#else
    window_info.SetAsPopup(nullptr, "Arkavo UI Generator");
#endif

    std::string url = "data:text/html,<html><body style='margin:0;padding:20px;font-family:system-ui;background:linear-gradient(135deg,%20%23667eea%200%25,%20%23764ba2%20100%25);color:%23fff;min-height:100vh;display:flex;align-items:center;justify-content:center;'><div style='text-align:center;'><h1 style='font-size:3em;margin:0;'>Arkavo UI Generator</h1><p style='font-size:1.5em;opacity:0.9;'>CEF Renderer Ready</p><p style='opacity:0.7;'>Waiting for AI-generated content...</p></div></body></html>";

    std::cout << "Creating browser (windowless mode)..." << std::endl;
    CefBrowserHost::CreateBrowser(window_info, client, url, browser_settings, nullptr, nullptr);

    CefRunMessageLoop();

    CefShutdown();

    std::cout << "Arkavo CEF Renderer shutdown" << std::endl;

    return 0;
}
