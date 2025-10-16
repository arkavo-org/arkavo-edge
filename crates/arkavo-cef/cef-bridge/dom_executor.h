#ifndef DOM_EXECUTOR_H
#define DOM_EXECUTOR_H

#include "include/cef_frame.h"
#include "uds_client.h"
#include <memory>
#include <string>

class DOMExecutor {
public:
    static DOMExecutor* GetInstance();

    void Initialize(CefRefPtr<CefFrame> frame, const std::string& socket_path);

    void ProcessCommand(const DOMCommand& cmd);

private:
    DOMExecutor() = default;
    ~DOMExecutor() = default;

    std::string EscapeJavaScript(const std::string& str);

    void ExecuteReplaceInnerHTML(uint32_t id, const std::string& selector, const std::string& html);
    void ExecuteSetAttribute(uint32_t id, const std::string& selector, const std::string& attr, const std::string& value);
    void ExecuteSetStyle(uint32_t id, const std::string& selector, const std::string& property, const std::string& value);
    void ExecuteSetTextContent(uint32_t id, const std::string& selector, const std::string& text);
    void ExecuteRemoveNode(uint32_t id, const std::string& selector);
    void ExecuteAddEventListener(uint32_t id, const std::string& selector, const std::string& event_type);

    void SendEvent(const DOMEvent& event);
    void SendFeedback(const DOMFeedback& feedback);

    CefRefPtr<CefFrame> frame_;
    std::unique_ptr<UdsClient> uds_client_;
    bool initialized_ = false;
};

#endif
