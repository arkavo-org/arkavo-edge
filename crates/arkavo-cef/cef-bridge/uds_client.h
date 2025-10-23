#ifndef UDS_CLIENT_H
#define UDS_CLIENT_H

#include <string>
#include <functional>
#include <thread>
#include <atomic>
#include <mutex>
#include <sys/socket.h>
#include <sys/un.h>

struct ConnectionState {
    int sock_fd;
    bool connected;

    ConnectionState() : sock_fd(-1), connected(false) {}
};

// Inline type definitions (normally in dom_executor.h, temporarily here for compilation)
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

struct DOMEvent {
    std::string event_type;
    std::string selector;
    std::string target_id;
    std::string value;
    std::string data;
};

struct DOMError {
    std::string error_type;
    std::string severity;
    std::string message;
    std::string source;
    uint32_t line;
};

class UdsClient {
public:
    explicit UdsClient(const std::string& socket_path);
    ~UdsClient();

    bool Bind();
    void Disconnect();

    void StartListening(std::function<void(const DOMCommand&)> callback);
    void StopListening();

    bool SendFeedback(const DOMFeedback& feedback);
    bool SendEvent(const DOMEvent& event);
    bool SendError(const DOMError& error);

private:
    void ListenLoop();
    void AcceptLoop();

    std::string socket_path_;
    ConnectionState conn_state_;
    int server_fd_;
    std::atomic<bool> running_;
    std::thread listen_thread_;
    std::thread accept_thread_;
    std::mutex conn_mutex_;
    std::function<void(const DOMCommand&)> command_callback_;
};

#endif
