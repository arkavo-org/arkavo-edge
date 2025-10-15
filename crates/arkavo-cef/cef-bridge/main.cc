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
#endif

int main(int argc, char* argv[]) {
    std::cout << ">>> main() entered <<<" << std::endl;
    std::cout.flush();

#ifdef __APPLE__
    // Load the CEF framework library at runtime instead of linking directly.
    // This is required on macOS.
    CefScopedLibraryLoader library_loader;
    if (!library_loader.LoadInMain()) {
        std::cerr << "Failed to load CEF framework!" << std::endl;
        return 1;
    }
    std::cout << ">>> CEF framework loaded <<<" << std::endl;
    std::cout.flush();
#endif

    CefMainArgs main_args(argc, argv);
    std::cout << ">>> CefMainArgs created <<<" << std::endl;
    std::cout.flush();

    // Parse socket path for browser process
    std::string socket_path = "/tmp/arkavo_dom.sock";
    std::cout << ">>> Parsing args <<<" << std::endl;
    std::cout.flush();
    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--socket" && i + 1 < argc) {
            socket_path = argv[++i];
        }
    }

    // Create app handler for both browser and subprocess
    std::cout << ">>> Creating ArkavoApp <<<" << std::endl;
    std::cout.flush();
    CefRefPtr<ArkavoApp> app = new ArkavoApp(socket_path);
    std::cout << ">>> ArkavoApp created <<<" << std::endl;
    std::cout.flush();

    // IMPORTANT: Handle subprocess execution first!
    // If this is a subprocess (--type=renderer, --type=gpu-process, etc.),
    // CefExecuteProcess will run the subprocess logic and return >= 0.
    // We must return immediately without touching CefSettings or CefInitialize.
    std::cout << ">>> Calling CefExecuteProcess <<<" << std::endl;
    std::cout.flush();
    int exit_code = CefExecuteProcess(main_args, app.get(), nullptr);
    std::cout << ">>> CefExecuteProcess returned: " << exit_code << " <<<" << std::endl;
    std::cout.flush();
    if (exit_code >= 0) {
        // This is a subprocess - exit immediately
        std::cout << ">>> Subprocess exiting with code: " << exit_code << " <<<" << std::endl;
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
    settings.windowless_rendering_enabled = false;

    // Enable verbose logging for debugging
    settings.log_severity = LOGSEVERITY_VERBOSE;
    CefString(&settings.log_file).FromASCII("/tmp/arkavo_cef_debug.log");

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
#ifdef __APPLE__
    CefRect bounds(0, 0, 1280, 800);
    window_info.SetAsChild(nullptr, bounds);
#else
    window_info.SetAsPopup(nullptr, "Arkavo UI Generator");
#endif

    CefBrowserSettings browser_settings;
    browser_settings.javascript = STATE_DISABLED;

    std::string url = "data:text/html,<html><body style='margin:0;padding:20px;font-family:system-ui;background:#1a1a1a;color:#fff;'><h1>Arkavo UI Generator</h1><p>Waiting for AI-generated content...</p></body></html>";

    std::cout << "Creating browser window..." << std::endl;
    CefBrowserHost::CreateBrowser(window_info, client, url, browser_settings, nullptr, nullptr);

    CefRunMessageLoop();

    CefShutdown();

    std::cout << "Arkavo CEF Renderer shutdown" << std::endl;

    return 0;
}
