#ifndef CEF_APP_H
#define CEF_APP_H

#include "include/cef_app.h"
#include "include/cef_client.h"

class ArkavoRenderProcessHandler : public CefRenderProcessHandler {
public:
    ArkavoRenderProcessHandler(const std::string& socket_path);

    void OnContextCreated(CefRefPtr<CefBrowser> browser,
                         CefRefPtr<CefFrame> frame,
                         CefRefPtr<CefV8Context> context) override;

    void OnContextReleased(CefRefPtr<CefBrowser> browser,
                          CefRefPtr<CefFrame> frame,
                          CefRefPtr<CefV8Context> context) override;

private:
    std::string socket_path_;

    IMPLEMENT_REFCOUNTING(ArkavoRenderProcessHandler);
};

class ArkavoApp : public CefApp {
public:
    ArkavoApp(const std::string& socket_path);

    CefRefPtr<CefRenderProcessHandler> GetRenderProcessHandler() override {
        return render_process_handler_;
    }

private:
    CefRefPtr<ArkavoRenderProcessHandler> render_process_handler_;

    IMPLEMENT_REFCOUNTING(ArkavoApp);
};

#endif
