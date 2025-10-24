#ifndef DOM_EXECUTOR_H
#define DOM_EXECUTOR_H

#include "include/cef_frame.h"
#include "uds_client.h"
#include <memory>
#include <string>

constexpr size_t MAX_HTML_PAYLOAD_SIZE = 1048576;
constexpr size_t MAX_CSS_PAYLOAD_SIZE = 102400;
constexpr size_t MAX_SELECTOR_LENGTH = 512;

class DOMExecutor {
public:
    static DOMExecutor* GetInstance();

    void Initialize(CefRefPtr<CefFrame> frame, const std::string& socket_path);
    void RegisterEventBridge();
    void PollEventQueue();

    void ProcessCommand(const DOMCommand& cmd);
    void HandleDOMEvent(const std::string& event_type, const std::string& selector,
                        const std::string& target_id, const std::string& value, const std::string& data);
    void SendError(const DOMError& error);

private:
    DOMExecutor() = default;
    ~DOMExecutor() = default;

    std::string EscapeJavaScript(const std::string& str);
    bool ValidateSelector(const std::string& selector);
    bool ValidatePayloadSize(const std::string& payload, size_t max_size);
    std::string SanitizeHtml(const std::string& html);
    void SendErrorFeedback(uint32_t id, const std::string& message);

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
