#include "uds_client.h"
#include "dom_executor.h"
#include <unistd.h>
#include <iostream>
#include <cstring>

UdsClient::UdsClient(const std::string& socket_path)
    : socket_path_(socket_path), sock_fd_(-1), running_(false) {
}

UdsClient::~UdsClient() {
    Disconnect();
}

bool UdsClient::Connect() {
    sock_fd_ = socket(AF_UNIX, SOCK_STREAM, 0);
    if (sock_fd_ < 0) {
        std::cerr << "Failed to create socket: " << strerror(errno) << std::endl;
        return false;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, socket_path_.c_str(), sizeof(addr.sun_path) - 1);

    if (connect(sock_fd_, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        std::cerr << "Failed to connect to socket: " << strerror(errno) << std::endl;
        close(sock_fd_);
        sock_fd_ = -1;
        return false;
    }

    std::cout << "Connected to UDS at " << socket_path_ << std::endl;
    return true;
}

void UdsClient::Disconnect() {
    StopListening();

    if (sock_fd_ >= 0) {
        close(sock_fd_);
        sock_fd_ = -1;
    }
}

void UdsClient::StartListening(std::function<void(const DOMCommand&)> callback) {
    command_callback_ = callback;
    running_ = true;

    listen_thread_ = std::thread([this]() {
        ListenLoop();
    });
}

void UdsClient::StopListening() {
    running_ = false;

    if (listen_thread_.joinable()) {
        listen_thread_.join();
    }
}

void UdsClient::ListenLoop() {
    uint8_t buffer[4096];

    while (running_) {
        uint32_t msg_len;
        ssize_t n = recv(sock_fd_, &msg_len, sizeof(msg_len), 0);

        if (n <= 0) {
            if (n < 0) {
                std::cerr << "Socket read error: " << strerror(errno) << std::endl;
            }
            break;
        }

        if (msg_len > sizeof(buffer)) {
            std::cerr << "Message too large: " << msg_len << std::endl;
            continue;
        }

        n = recv(sock_fd_, buffer, msg_len, 0);

        if (n < 0) {
            std::cerr << "Socket read error: " << strerror(errno) << std::endl;
            break;
        }

        if (n != msg_len) {
            std::cerr << "Incomplete message: expected " << msg_len << ", got " << n << std::endl;
            continue;
        }

        DOMCommand cmd;
        cmd.id = 0;
        cmd.op = DOMOp::ReplaceInnerHTML;

        if (command_callback_) {
            command_callback_(cmd);
        }
    }
}

bool UdsClient::SendFeedback(const DOMFeedback& feedback) {
    if (sock_fd_ < 0) {
        return false;
    }

    uint8_t buffer[1024];
    uint32_t offset = 0;

    memcpy(buffer + offset, &feedback.id, sizeof(feedback.id));
    offset += sizeof(feedback.id);

    memcpy(buffer + offset, &feedback.status, sizeof(feedback.status));
    offset += sizeof(feedback.status);

    memcpy(buffer + offset, &feedback.exec_time_ns, sizeof(feedback.exec_time_ns));
    offset += sizeof(feedback.exec_time_ns);

    uint32_t msg_len_offset = offset;
    uint32_t msg_len = 0;
    offset += sizeof(msg_len);

    size_t msg_size = feedback.message.size();
    memcpy(buffer + offset, feedback.message.c_str(), msg_size);
    offset += msg_size;

    msg_len = offset - msg_len_offset - sizeof(msg_len);
    memcpy(buffer + msg_len_offset, &msg_len, sizeof(msg_len));

    uint32_t frame_len = offset;
    ssize_t n = send(sock_fd_, &frame_len, sizeof(frame_len), 0);
    if (n < 0) {
        std::cerr << "Failed to send frame length: " << strerror(errno) << std::endl;
        return false;
    }

    n = send(sock_fd_, buffer, offset, 0);
    if (n < 0) {
        std::cerr << "Failed to send feedback: " << strerror(errno) << std::endl;
        return false;
    }

    return true;
}
