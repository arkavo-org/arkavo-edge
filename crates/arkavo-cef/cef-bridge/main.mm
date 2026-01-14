#include "cef_app.h"
#include "browser_client.h"
#include "dom_executor.h"
#include "uds_client.h"
#include "include/cef_app.h"
#include "include/cef_command_line.h"
#include "include/wrapper/cef_helpers.h"
#include <algorithm>
#include <iostream>
#include <string>
#include <climits>

#ifdef __APPLE__
#include <mach-o/dyld.h>
#include "include/wrapper/cef_library_loader.h"
#include <Cocoa/Cocoa.h>

// Window delegate to handle close button
@interface WindowDelegate : NSObject <NSWindowDelegate> {
    CefRefPtr<CefBrowser> _browser;
    NSWindow* _window;
    BOOL _isClosing;
}
- (void)setBrowser:(CefRefPtr<CefBrowser>)browser;
- (void)setWindow:(NSWindow*)window;
@end

@implementation WindowDelegate
- (instancetype)init {
    self = [super init];
    if (self) {
        _isClosing = NO;
    }
    return self;
}

- (void)setBrowser:(CefRefPtr<CefBrowser>)browser {
    _browser = browser;
}

- (void)setWindow:(NSWindow*)window {
    _window = window;
}

- (BOOL)windowShouldClose:(NSWindow*)window {
    if (_isClosing) {
        return NO;
    }

    _isClosing = YES;
    std::cout << "Window close button clicked - initiating shutdown..." << std::endl;

    // Hide window immediately for better UX
    [window orderOut:nil];

    if (_browser) {
        _browser->GetHost()->CloseBrowser(true);

        // Send shutdown signal async after tiny delay to not block window fade
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.05 * NSEC_PER_SEC)),
                      dispatch_get_main_queue(), ^{
            // Send shutdown signal to Rust
            DOMError shutdown_signal;
            shutdown_signal.error_type = "shutdown";
            shutdown_signal.severity = "info";
            shutdown_signal.message = "CEF window closed by user";
            shutdown_signal.source = "main.mm";
            shutdown_signal.line = 0;
            DOMExecutor::GetInstance()->SendError(shutdown_signal);
            std::cout << "Shutdown signal sent to Rust" << std::endl;
        });

        // Give CEF 500ms to clean up, then quit message loop
        // Do NOT call exit() - let CefRunMessageLoop() return naturally
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.5 * NSEC_PER_SEC)),
                      dispatch_get_main_queue(), ^{
            std::cout << "Quitting CEF message loop..." << std::endl;
            CefQuitMessageLoop();
        });
    } else {
        // Send shutdown signal async
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.05 * NSEC_PER_SEC)),
                      dispatch_get_main_queue(), ^{
            DOMError shutdown_signal;
            shutdown_signal.error_type = "shutdown";
            shutdown_signal.severity = "info";
            shutdown_signal.message = "CEF window closed by user";
            shutdown_signal.source = "main.mm";
            shutdown_signal.line = 0;
            DOMExecutor::GetInstance()->SendError(shutdown_signal);
            std::cout << "Shutdown signal sent to Rust" << std::endl;
        });

        // Give Rust time to process shutdown signal, then quit message loop
        // Do NOT call exit() - let CefRunMessageLoop() return naturally
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.5 * NSEC_PER_SEC)),
                      dispatch_get_main_queue(), ^{
            std::cout << "Quitting CEF message loop..." << std::endl;
            CefQuitMessageLoop();
        });
    }
    return NO;
}
@end

// Global delegate reference
static WindowDelegate* g_windowDelegate = nil;

// C function to set browser in window delegate (callable from C++)
extern "C" void SetBrowserInWindowDelegate(CefRefPtr<CefBrowser> browser) {
    if (g_windowDelegate) {
        [g_windowDelegate setBrowser:browser];
    }
}
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
    settings.windowless_rendering_enabled = false;
#else
    settings.windowless_rendering_enabled = false;
#endif

    // Set unique cache path per instance to avoid singleton lock conflicts
    std::string cache_id = socket_path;
    std::replace(cache_id.begin(), cache_id.end(), '/', '_');
    std::replace(cache_id.begin(), cache_id.end(), '.', '_');
    std::string cache_path = "/tmp/arkavo_cef_cache_" + cache_id;
    CefString(&settings.cache_path).FromASCII(cache_path.c_str());

    // Suppress Chromium internal errors
    settings.log_severity = LOGSEVERITY_FATAL;
    std::string log_file = "/tmp/arkavo_cef_" + cache_id + ".log";
    CefString(&settings.log_file).FromASCII(log_file.c_str());

    // Disable background networking
    CefString(&settings.user_agent_product).FromASCII("ArkavoEdge/1.0");

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
    browser_settings.javascript = STATE_ENABLED;

#ifdef __APPLE__
    // Create a visible window on macOS
    NSRect frame = NSMakeRect(100, 100, 1024, 768);
    NSWindow* window = [[NSWindow alloc] initWithContentRect:frame
                                         styleMask:(NSWindowStyleMaskTitled |
                                                   NSWindowStyleMaskClosable |
                                                   NSWindowStyleMaskMiniaturizable |
                                                   NSWindowStyleMaskResizable)
                                         backing:NSBackingStoreBuffered
                                         defer:NO];
    [window setTitle:@"Arkavo Edge"];

    // Enable the close button to quit the app
    [window setReleasedWhenClosed:NO];

    // Create and set window delegate to handle close button
    g_windowDelegate = [[WindowDelegate alloc] init];
    [g_windowDelegate setWindow:window];
    [window setDelegate:g_windowDelegate];

    [window makeKeyAndOrderFront:nil];

    // Create Edit menu with copy/paste support
    NSMenu* mainMenu = [[NSMenu alloc] init];

    // Edit menu
    NSMenuItem* editMenuItem = [[NSMenuItem alloc] init];
    NSMenu* editMenu = [[NSMenu alloc] initWithTitle:@"Edit"];

    [editMenu addItemWithTitle:@"Cut" action:@selector(cut:) keyEquivalent:@"x"];
    [editMenu addItemWithTitle:@"Copy" action:@selector(copy:) keyEquivalent:@"c"];
    [editMenu addItemWithTitle:@"Paste" action:@selector(paste:) keyEquivalent:@"v"];
    [editMenu addItem:[NSMenuItem separatorItem]];
    [editMenu addItemWithTitle:@"Select All" action:@selector(selectAll:) keyEquivalent:@"a"];

    [editMenuItem setSubmenu:editMenu];
    [mainMenu addItem:editMenuItem];

    [NSApp setMainMenu:mainMenu];

    NSView* contentView = [window contentView];
    CefRect rect(0, 0, 1024, 768);
    window_info.SetAsChild(contentView, rect);
#else
    window_info.SetAsPopup(nullptr, "Arkavo Edge");
#endif

    std::string url = "data:text/html,<html><head><style>"
        "body { margin:0; padding:0; font-family:system-ui; height:100vh; display:flex; flex-direction:column; }"
        "#content { flex:1; overflow:auto; padding:20px; background:linear-gradient(135deg, #667eea 0%, #764ba2 100%); color:#fff; }"
        "#prompt-bar { position:fixed; bottom:0; left:0; right:0; background:#fff; padding:12px; box-shadow:0 -2px 8px rgba(0,0,0,0.1); display:flex; gap:8px; }"
        "#prompt-input { flex:1; padding:10px; border:1px solid #ddd; border-radius:4px; font-size:14px; }"
        "#prompt-submit { padding:10px 20px; background:#667eea; color:#fff; border:none; border-radius:4px; cursor:pointer; font-size:14px; font-weight:600; }"
        "#prompt-submit:hover { background:#5568d3; }"
        ".loading { text-align:center; padding-top:30vh; }"
        ".spinner { width:40px; height:40px; border:3px solid rgba(255,255,255,0.3); border-top:3px solid #fff; border-radius:50%; animation:spin 1s linear infinite; margin:0 auto 20px; }"
        "@keyframes spin { to { transform:rotate(360deg); } }"
        "@keyframes pulse { 0%,100% { opacity:1; } 50% { opacity:0.5; } }"
        "</style></head><body>"
        "<div id='content'>"
        "<div class='loading'>"
        "<div class='spinner'></div>"
        "<p style='font-size:1.2em;'>Loading Arkavo Edge...</p>"
        "</div>"
        "</div>"
        "<div id='prompt-bar'>"
        "<input type='text' id='prompt-input' placeholder='Enter your prompt...' />"
        "<button id='prompt-submit'>Submit</button>"
        "</div>"
        "</body></html>";

    std::cout << "Creating browser (windowed mode)..." << std::endl;
    CefBrowserHost::CreateBrowser(window_info, client, url, browser_settings, nullptr, nullptr);

    CefRunMessageLoop();

    // Shutdown DOMExecutor BEFORE CefShutdown to avoid race conditions
    // This stops the UDS client threads cleanly before CEF tears down
    std::cout << "Shutting down DOMExecutor..." << std::endl;
    DOMExecutor::GetInstance()->Shutdown();

    CefShutdown();

    std::cout << "Arkavo CEF Renderer shutdown complete" << std::endl;

    return 0;
}
