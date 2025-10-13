#ifndef DOM_EXECUTOR_H
#define DOM_EXECUTOR_H

#include "include/cef_frame.h"
#include "include/cef_dom.h"
#include "uds_client.h"
#include <memory>
#include <string>

enum class DOMOp : uint8_t {
    ReplaceInnerHTML = 0,
    SetAttribute = 1,
    SetStyle = 2,
    RemoveNode = 3,
    AppendNode = 4,
    QuerySelector = 5,
    AddEventListener = 6,
    SetTextContent = 7,
};

struct DOMCommand {
    uint32_t id;
    DOMOp op;
    std::string selector;
    std::string payload;
    std::string property;
};

struct DOMFeedback {
    uint32_t id;
    uint8_t status;
    uint64_t exec_time_ns;
    std::string message;
};

class DOMExecutor {
public:
    static DOMExecutor* GetInstance();

    void Initialize(CefRefPtr<CefFrame> frame, const std::string& socket_path);

    void ProcessCommand(const DOMCommand& cmd);

private:
    DOMExecutor() = default;
    ~DOMExecutor() = default;

    CefRefPtr<CefDOMNode> FindNode(const std::string& selector);

    void ExecuteReplaceInnerHTML(uint32_t id, const std::string& selector, const std::string& html);
    void ExecuteSetAttribute(uint32_t id, const std::string& selector, const std::string& attr, const std::string& value);
    void ExecuteSetStyle(uint32_t id, const std::string& selector, const std::string& property, const std::string& value);
    void ExecuteSetTextContent(uint32_t id, const std::string& selector, const std::string& text);
    void ExecuteRemoveNode(uint32_t id, const std::string& selector);

    void SendFeedback(const DOMFeedback& feedback);

    CefRefPtr<CefFrame> frame_;
    std::unique_ptr<UdsClient> uds_client_;
};

#endif
