#include "browser_client.h"
#include "dom_executor.h"
#include "include/cef_app.h"
#include "include/wrapper/cef_helpers.h"
#include <iostream>
#include <fstream>
#include <sstream>
#include <ctime>

#ifdef __APPLE__
// Forward declare the function to set browser in window delegate
extern "C" void SetBrowserInWindowDelegate(CefRefPtr<CefBrowser> browser);
#endif

ArkavoBrowserClient::ArkavoBrowserClient(const std::string& socket_path)
    : socket_path_(socket_path), dom_executor_initialized_(false), screenshot_saved_(false) {
}

void ArkavoBrowserClient::OnAfterCreated(CefRefPtr<CefBrowser> browser) {
    browser_ = browser;
    std::cout << "Browser window created" << std::endl;

#ifdef __APPLE__
    // Set browser reference in window delegate so close button works
    SetBrowserInWindowDelegate(browser);
#endif

    if (!dom_executor_initialized_) {
        auto frame = browser->GetMainFrame();
        if (frame) {
            DOMExecutor::GetInstance()->Initialize(frame, socket_path_);
            dom_executor_initialized_ = true;
            std::cout << "DOMExecutor initialized in browser process" << std::endl;
        }
    }
}

bool ArkavoBrowserClient::DoClose(CefRefPtr<CefBrowser> browser) {
    std::cout << "DoClose called - allowing browser to close" << std::endl;
    return false;
}

void ArkavoBrowserClient::OnBeforeClose(CefRefPtr<CefBrowser> browser) {
    browser_ = nullptr;
    std::cout << "OnBeforeClose called - quitting message loop..." << std::endl;
    CefQuitMessageLoop();
    std::cout << "Message loop quit requested" << std::endl;
}

void ArkavoBrowserClient::OnBeforeContextMenu(CefRefPtr<CefBrowser> browser,
                                              CefRefPtr<CefFrame> frame,
                                              CefRefPtr<CefContextMenuParams> params,
                                              CefRefPtr<CefMenuModel> model) {
    model->Clear();
}

void ArkavoBrowserClient::OnLoadEnd(CefRefPtr<CefBrowser> browser,
                                    CefRefPtr<CefFrame> frame,
                                    int httpStatusCode) {
    if (frame->IsMain()) {
        std::cout << "Page loaded successfully" << std::endl;

        std::ostringstream js;
        js << "(function() {"
           << "  var input = document.getElementById('prompt-input');"
           << "  var button = document.getElementById('prompt-submit');"
           << "  "
           << "  function submitPrompt() {"
           << "    var value = input.value.trim();"
           << "    if (value.length === 0) return;"
           << "    "
           << "    if (typeof window.ArkavoEventBridge === 'function') {"
           << "      window.ArkavoEventBridge({"
           << "        event_type: 'submit',"
           << "        selector: '#prompt-input',"
           << "        target_id: 'prompt-input',"
           << "        value: value,"
           << "        data: '{}'"
           << "      });"
           << "      input.value = '';"
           << "    } else {"
           << "      console.error('ArkavoEventBridge not available');"
           << "    }"
           << "  }"
           << "  "
           << "  button.addEventListener('click', submitPrompt);"
           << "  input.addEventListener('keypress', function(e) {"
           << "    if (e.key === 'Enter') submitPrompt();"
           << "  });"
           << "  "
           << "  console.log('Prompt bar event listeners registered');"
           << "})();";

        frame->ExecuteJavaScript(js.str(), frame->GetURL(), 0);
        std::cout << "Prompt bar event listeners initialized" << std::endl;
    }
}

void ArkavoBrowserClient::GetViewRect(CefRefPtr<CefBrowser> browser, CefRect& rect) {
    rect.x = 0;
    rect.y = 0;
    rect.width = 1024;
    rect.height = 768;
}

void ArkavoBrowserClient::OnPaint(CefRefPtr<CefBrowser> browser,
                                   PaintElementType type,
                                   const RectList& dirtyRects,
                                   const void* buffer,
                                   int width,
                                   int height) {
    std::cout << "OnPaint called: " << width << "x" << height
              << " (" << dirtyRects.size() << " dirty rects)" << std::endl;

    if (!screenshot_saved_ && buffer && width > 0 && height > 0) {
        SaveScreenshot(buffer, width, height);
        screenshot_saved_ = true;
    }
}

void ArkavoBrowserClient::SaveScreenshot(const void* buffer, int width, int height) {
    std::time_t now = std::time(nullptr);
    std::string ppm_filename = "/tmp/arkavo_cef_screenshot_" + std::to_string(now) + ".ppm";
    std::string png_filename = "/tmp/arkavo_cef_screenshot_" + std::to_string(now) + ".png";

    // Save as PPM first
    std::ofstream file(ppm_filename, std::ios::binary);
    if (!file.is_open()) {
        std::cerr << "Failed to open screenshot file: " << ppm_filename << std::endl;
        return;
    }

    file << "P6\n" << width << " " << height << "\n255\n";

    const uint8_t* pixels = static_cast<const uint8_t*>(buffer);

    for (int y = 0; y < height; y++) {
        for (int x = 0; x < width; x++) {
            int idx = (y * width + x) * 4;
            uint8_t b = pixels[idx + 0];
            uint8_t g = pixels[idx + 1];
            uint8_t r = pixels[idx + 2];

            file.write(reinterpret_cast<const char*>(&r), 1);
            file.write(reinterpret_cast<const char*>(&g), 1);
            file.write(reinterpret_cast<const char*>(&b), 1);
        }
    }

    file.close();
    std::cout << "Screenshot saved to: " << ppm_filename << std::endl;

#ifdef __APPLE__
    // Convert PPM to PNG using macOS sips command
    std::string convert_cmd = "sips -s format png " + ppm_filename + " --out " + png_filename + " 2>/dev/null";
    int result = std::system(convert_cmd.c_str());

    if (result == 0) {
        std::cout << "PNG screenshot saved to: " << png_filename << std::endl;

        // Remove the PPM file since we have PNG now
        std::remove(ppm_filename.c_str());
    } else {
        std::cerr << "Failed to convert to PNG, PPM file retained" << std::endl;
    }
#endif
}
