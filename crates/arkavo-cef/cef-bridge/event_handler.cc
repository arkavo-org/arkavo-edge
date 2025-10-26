#include "event_handler.h"
#include "dom_executor.h"
#include <iostream>

ArkavoEventHandler::ArkavoEventHandler() {
}

bool ArkavoEventHandler::Execute(const CefString& name,
                                 CefRefPtr<CefV8Value> object,
                                 const CefV8ValueList& arguments,
                                 CefRefPtr<CefV8Value>& retval,
                                 CefString& exception) {
    if (name == "arkavoPushEvent" && arguments.size() == 1 && arguments[0]->IsObject()) {
        CefRefPtr<CefV8Value> eventObj = arguments[0];

        std::string event_type = eventObj->GetValue("event_type")->GetStringValue().ToString();
        std::string selector = eventObj->GetValue("selector")->GetStringValue().ToString();
        std::string target_id = eventObj->GetValue("target_id")->GetStringValue().ToString();
        std::string value = eventObj->GetValue("value")->GetStringValue().ToString();
        std::string data = eventObj->GetValue("data")->GetStringValue().ToString();

        SendEventToRust(event_type, selector, target_id, value, data);

        retval = CefV8Value::CreateBool(true);
        return true;
    }

    return false;
}

void ArkavoEventHandler::SendEventToRust(const std::string& event_type,
                                         const std::string& selector,
                                        const std::string& target_id,
                                         const std::string& value,
                                         const std::string& data) {
    DOMExecutor::GetInstance()->HandleDOMEvent(event_type, selector, target_id, value, data);
}
