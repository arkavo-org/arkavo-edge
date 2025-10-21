#ifndef EVENT_HANDLER_H
#define EVENT_HANDLER_H

#include "include/cef_v8.h"
#include <string>

class ArkavoEventHandler : public CefV8Handler {
public:
    ArkavoEventHandler();

    bool Execute(const CefString& name,
                CefRefPtr<CefV8Value> object,
                const CefV8ValueList& arguments,
                CefRefPtr<CefV8Value>& retval,
                CefString& exception) override;

private:
    void SendEventToRust(const std::string& event_type,
                        const std::string& selector,
                        const std::string& target_id,
                        const std::string& value,
                        const std::string& data);

    IMPLEMENT_REFCOUNTING(ArkavoEventHandler);
};

#endif
