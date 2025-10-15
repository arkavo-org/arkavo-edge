#ifndef BROWSER_CLIENT_H
#define BROWSER_CLIENT_H

#include "include/cef_client.h"
#include "include/cef_life_span_handler.h"
#include "include/cef_load_handler.h"
#include <string>

class ArkavoBrowserClient : public CefClient,
                            public CefLifeSpanHandler,
                            public CefLoadHandler {
public:
    ArkavoBrowserClient(const std::string& socket_path);

    CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override {
        return this;
    }

    CefRefPtr<CefLoadHandler> GetLoadHandler() override {
        return this;
    }

    void OnAfterCreated(CefRefPtr<CefBrowser> browser) override;
    bool DoClose(CefRefPtr<CefBrowser> browser) override;
    void OnBeforeClose(CefRefPtr<CefBrowser> browser) override;
    void OnLoadEnd(CefRefPtr<CefBrowser> browser,
                   CefRefPtr<CefFrame> frame,
                   int httpStatusCode) override;

private:
    std::string socket_path_;
    CefRefPtr<CefBrowser> browser_;

    IMPLEMENT_REFCOUNTING(ArkavoBrowserClient);
};

#endif
