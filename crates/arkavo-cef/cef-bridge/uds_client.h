#ifndef UDS_CLIENT_H
#define UDS_CLIENT_H

#include <string>
#include <functional>
#include <thread>
#include <atomic>
#include <sys/socket.h>
#include <sys/un.h>

struct DOMCommand;
struct DOMFeedback;

class UdsClient {
public:
    explicit UdsClient(const std::string& socket_path);
    ~UdsClient();

    bool Connect();
    void Disconnect();

    void StartListening(std::function<void(const DOMCommand&)> callback);
    void StopListening();

    bool SendFeedback(const DOMFeedback& feedback);

private:
    void ListenLoop();

    std::string socket_path_;
    int sock_fd_;
    std::atomic<bool> running_;
    std::thread listen_thread_;
    std::function<void(const DOMCommand&)> command_callback_;
};

#endif
