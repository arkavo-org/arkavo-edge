#ifndef BROWSER_CLIENT_H
#define BROWSER_CLIENT_H

#include "include/cef_client.h"
#include "include/cef_context_menu_handler.h"
#include "include/cef_life_span_handler.h"
#include "include/cef_load_handler.h"
#include "include/cef_render_handler.h"
#include <string>

class ArkavoBrowserClient : public CefClient,
                            public CefContextMenuHandler,
                            public CefLifeSpanHandler,
                            public CefLoadHandler,
                            public CefRenderHandler {
public:
    ArkavoBrowserClient(const std::string& socket_path);

    CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override {
        return this;
    }

    CefRefPtr<CefLoadHandler> GetLoadHandler() override {
        return this;
    }

    CefRefPtr<CefRenderHandler> GetRenderHandler() override {
        return this;
    }

    CefRefPtr<CefContextMenuHandler> GetContextMenuHandler() override {
        return this;
    }

    void OnBeforeContextMenu(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            CefRefPtr<CefContextMenuParams> params,
                            CefRefPtr<CefMenuModel> model) override;

    bool OnContextMenuCommand(CefRefPtr<CefBrowser> browser,
                             CefRefPtr<CefFrame> frame,
                             CefRefPtr<CefContextMenuParams> params,
                             int command_id,
                             EventFlags event_flags) override;

    void GetViewRect(CefRefPtr<CefBrowser> browser, CefRect& rect) override;
    void OnPaint(CefRefPtr<CefBrowser> browser,
                 PaintElementType type,
                 const RectList& dirtyRects,
                 const void* buffer,
                 int width,
                 int height) override;

    void OnAfterCreated(CefRefPtr<CefBrowser> browser) override;
    bool DoClose(CefRefPtr<CefBrowser> browser) override;
    void OnBeforeClose(CefRefPtr<CefBrowser> browser) override;
    void OnLoadEnd(CefRefPtr<CefBrowser> browser,
                   CefRefPtr<CefFrame> frame,
                   int httpStatusCode) override;

    void SaveScreenshot(const void* buffer, int width, int height);

private:
    std::string socket_path_;
    CefRefPtr<CefBrowser> browser_;
    bool dom_executor_initialized_;
    bool screenshot_saved_;

    IMPLEMENT_REFCOUNTING(ArkavoBrowserClient);
};

#endif
