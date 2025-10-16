#include "browser_client.h"
#include "dom_executor.h"
#include "include/cef_app.h"
#include "include/wrapper/cef_helpers.h"
#include <iostream>
#include <fstream>
#include <ctime>

ArkavoBrowserClient::ArkavoBrowserClient(const std::string& socket_path)
    : socket_path_(socket_path), dom_executor_initialized_(false), screenshot_saved_(false) {
}

void ArkavoBrowserClient::OnAfterCreated(CefRefPtr<CefBrowser> browser) {
    browser_ = browser;
    std::cout << "Browser window created" << std::endl;

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
    return false;
}

void ArkavoBrowserClient::OnBeforeClose(CefRefPtr<CefBrowser> browser) {
    browser_ = nullptr;
    std::cout << "Browser window closing..." << std::endl;
    CefQuitMessageLoop();
}

void ArkavoBrowserClient::OnLoadEnd(CefRefPtr<CefBrowser> browser,
                                    CefRefPtr<CefFrame> frame,
                                    int httpStatusCode) {
    if (frame->IsMain()) {
        std::cout << "Page loaded successfully" << std::endl;
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
    std::string filename = "/tmp/arkavo_cef_screenshot_" + std::to_string(now) + ".ppm";

    std::ofstream file(filename, std::ios::binary);
    if (!file.is_open()) {
        std::cerr << "Failed to open screenshot file: " << filename << std::endl;
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
    std::cout << "Screenshot saved to: " << filename << std::endl;
}
