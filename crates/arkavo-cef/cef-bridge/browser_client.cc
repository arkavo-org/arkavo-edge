#include "browser_client.h"
#include "include/cef_app.h"
#include "include/wrapper/cef_helpers.h"
#include <iostream>

ArkavoBrowserClient::ArkavoBrowserClient(const std::string& socket_path)
    : socket_path_(socket_path) {
}

void ArkavoBrowserClient::OnAfterCreated(CefRefPtr<CefBrowser> browser) {
    browser_ = browser;
    std::cout << "Browser window created" << std::endl;
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
