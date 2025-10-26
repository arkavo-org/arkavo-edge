#include "uds_client.h"
// #include "dom_executor.h" - Temporarily disabled due to CEF API compatibility
#include <unistd.h>
#include <iostream>
#include <cstring>
#include <sys/select.h>
#include <sys/stat.h>
#include <cerrno>

UdsClient::UdsClient(const std::string& socket_path)
    : socket_path_(socket_path), conn_state_(), server_fd_(-1), running_(false) {
}

UdsClient::~UdsClient() {
    Disconnect();
}

bool UdsClient::Bind() {
    if (socket_path_.find("/tmp/arkavo_") != 0 &&
        socket_path_.find("/private/tmp/arkavo_") != 0 &&
        socket_path_.find("/var/folders/") != 0) {
        std::cerr << "Invalid socket path: must start with /tmp/arkavo_, /private/tmp/arkavo_, or /var/folders/" << std::endl;
        return false;
    }

    if (socket_path_.length() > 100) {
        std::cerr << "Socket path too long (max 100 characters)" << std::endl;
        return false;
    }

    server_fd_ = socket(AF_UNIX, SOCK_STREAM, 0);
    if (server_fd_ < 0) {
        std::cerr << "Failed to create socket: " << strerror(errno) << std::endl;
        return false;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, socket_path_.c_str(), sizeof(addr.sun_path) - 1);

    unlink(socket_path_.c_str());

    if (bind(server_fd_, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        std::cerr << "Failed to bind socket: " << strerror(errno) << std::endl;
        close(server_fd_);
        server_fd_ = -1;
        return false;
    }

    if (listen(server_fd_, 1) < 0) {
        std::cerr << "Failed to listen on socket: " << strerror(errno) << std::endl;
        close(server_fd_);
        server_fd_ = -1;
        return false;
    }

    if (chmod(socket_path_.c_str(), 0600) < 0) {
        std::cerr << "Failed to set socket permissions: " << strerror(errno) << std::endl;
        close(server_fd_);
        server_fd_ = -1;
        return false;
    }

    std::cout << "UDS server listening at " << socket_path_ << " (permissions: 0600)" << std::endl;

    accept_thread_ = std::thread([this]() {
        AcceptLoop();
    });

    return true;
}

void UdsClient::AcceptLoop() {
    // Set timeout for accept (30 seconds)
    fd_set readfds;
    struct timeval timeout;

    FD_ZERO(&readfds);
    FD_SET(server_fd_, &readfds);

    timeout.tv_sec = 30;
    timeout.tv_usec = 0;

    int result = select(server_fd_ + 1, &readfds, NULL, NULL, &timeout);

    if (result < 0) {
        std::cerr << "Select failed: " << strerror(errno) << std::endl;
        return;
    }

    if (result == 0) {
        std::cerr << "Accept timeout after 30 seconds - no client connected" << std::endl;
        return;
    }

    int client_fd = accept(server_fd_, NULL, NULL);
    if (client_fd < 0) {
        std::cerr << "Failed to accept connection: " << strerror(errno) << std::endl;
        return;
    }

    {
        std::lock_guard<std::mutex> lock(conn_mutex_);
        conn_state_.sock_fd = client_fd;
        conn_state_.connected = true;
    }

    close(server_fd_);
    server_fd_ = -1;

    std::cout << "UDS client connected" << std::endl;

    // Flush any queued errors
    FlushErrorQueue();
}

void UdsClient::Disconnect() {
    StopListening();

    if (server_fd_ >= 0) {
        close(server_fd_);
        server_fd_ = -1;
    }

    if (accept_thread_.joinable()) {
        accept_thread_.join();
    }

    {
        std::lock_guard<std::mutex> lock(conn_mutex_);
        if (conn_state_.sock_fd >= 0) {
            close(conn_state_.sock_fd);
            conn_state_.sock_fd = -1;
            conn_state_.connected = false;
        }
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

    // Wait for connection (with timeout)
    for (int i = 0; i < 100 && running_; i++) {
        {
            std::lock_guard<std::mutex> lock(conn_mutex_);
            if (conn_state_.connected && conn_state_.sock_fd >= 0) {
                break;
            }
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
    }

    while (running_) {
        int socket_fd;
        {
            std::lock_guard<std::mutex> lock(conn_mutex_);
            if (!conn_state_.connected || conn_state_.sock_fd < 0) {
                break;
            }
            socket_fd = conn_state_.sock_fd;
        }

        uint32_t msg_len;
        ssize_t n = recv(socket_fd, &msg_len, sizeof(msg_len), 0);

        if (n <= 0) {
            if (n < 0) {
                std::cerr << "Socket read error: " << strerror(errno) << std::endl;
            }
            break;
        }

        if (msg_len > sizeof(buffer)) {
            continue;
        }

        n = recv(socket_fd, buffer, msg_len, 0);

        if (n < 0) {
            std::cerr << "Socket read error: " << strerror(errno) << std::endl;
            break;
        }

        if (n != msg_len) {
            std::cerr << "Incomplete message: expected " << msg_len << ", got " << n << std::endl;
            continue;
        }

        // Deserialize the command from buffer
        DOMCommand cmd;
        uint32_t offset = 0;

        // Read id (4 bytes)
        if (offset + 4 > msg_len) continue;
        memcpy(&cmd.id, &buffer[offset], 4);
        offset += 4;

        // Read op (1 byte)
        if (offset + 1 > msg_len) continue;
        cmd.op = static_cast<DOMOp>(buffer[offset]);
        offset += 1;

        // Read selector (length-prefixed string)
        if (offset + 4 > msg_len) continue;
        uint32_t selector_len;
        memcpy(&selector_len, &buffer[offset], 4);
        offset += 4;
        if (offset + selector_len > msg_len) continue;
        cmd.selector = std::string(reinterpret_cast<char*>(&buffer[offset]), selector_len);
        offset += selector_len;

        // Read payload (length-prefixed string)
        if (offset + 4 > msg_len) continue;
        uint32_t payload_len;
        memcpy(&payload_len, &buffer[offset], 4);
        offset += 4;
        if (offset + payload_len > msg_len) continue;
        cmd.payload = std::string(reinterpret_cast<char*>(&buffer[offset]), payload_len);
        offset += payload_len;

        // Read property (length-prefixed string, optional)
        if (offset + 4 > msg_len) continue;
        uint32_t property_len;
        memcpy(&property_len, &buffer[offset], 4);
        offset += 4;
        if (property_len > 0 && offset + property_len <= msg_len) {
            cmd.property = std::string(reinterpret_cast<char*>(&buffer[offset]), property_len);
        }

        if (command_callback_) {
            command_callback_(cmd);
        }
    }
}

bool UdsClient::SendFeedback(const DOMFeedback& feedback) {
    std::lock_guard<std::mutex> lock(conn_mutex_);

    if (!conn_state_.connected || conn_state_.sock_fd < 0) {
        std::cerr << "Cannot send feedback: not connected" << std::endl;
        return false;
    }

    uint8_t buffer[1024];
    uint32_t offset = 0;

    // Message type 0x01 for feedback
    buffer[offset++] = 0x01;

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
    ssize_t n = send(conn_state_.sock_fd, &frame_len, sizeof(frame_len), 0);
    if (n < 0) {
        std::cerr << "Failed to send frame length: " << strerror(errno) << std::endl;
        return false;
    }

    n = send(conn_state_.sock_fd, buffer, offset, 0);
    if (n < 0) {
        std::cerr << "Failed to send feedback: " << strerror(errno) << std::endl;
        return false;
    }

    return true;
}

bool UdsClient::SendEvent(const DOMEvent& event) {
    std::lock_guard<std::mutex> lock(conn_mutex_);

    std::cerr << "[UDS] SendEvent called - connected=" << conn_state_.connected
              << " fd=" << conn_state_.sock_fd << std::endl;

    if (!conn_state_.connected || conn_state_.sock_fd < 0) {
        std::cerr << "[UDS ERROR] Cannot send event: not connected" << std::endl;
        return false;
    }

    uint8_t buffer[2048];
    uint32_t offset = 0;

    uint8_t msg_type = 0x02;
    buffer[offset++] = msg_type;

    auto write_string = [&](const std::string& str) {
        uint32_t len = str.size();
        memcpy(buffer + offset, &len, sizeof(len));
        offset += sizeof(len);
        memcpy(buffer + offset, str.c_str(), len);
        offset += len;
    };

    write_string(event.event_type);
    write_string(event.selector);
    write_string(event.target_id);
    write_string(event.value);
    write_string(event.data);

    uint32_t frame_len = offset;
    std::cerr << "[UDS] Sending frame_len=" << frame_len << " bytes (payload=" << offset << " bytes)" << std::endl;

    ssize_t n = send(conn_state_.sock_fd, &frame_len, sizeof(frame_len), 0);
    if (n < 0) {
        std::cerr << "[UDS ERROR] Failed to send event frame length: " << strerror(errno) << std::endl;
        return false;
    }
    std::cerr << "[UDS] Sent frame_len header: " << n << " bytes" << std::endl;

    n = send(conn_state_.sock_fd, buffer, offset, 0);
    if (n < 0) {
        std::cerr << "[UDS ERROR] Failed to send event: " << strerror(errno) << std::endl;
        return false;
    }
    std::cerr << "[UDS] Sent event payload: " << n << " bytes (event_type="
              << event.event_type << ", value=" << event.value << ")" << std::endl;

    return true;
}

void UdsClient::FlushErrorQueue() {
    std::lock_guard<std::mutex> queue_lock(error_queue_mutex_);

    size_t flushed_count = 0;

    while (!error_queue_.empty()) {
        const DOMError& queued_error = error_queue_.front();

        // Try to send without taking the queue lock again
        {
            std::lock_guard<std::mutex> conn_lock(conn_mutex_);
            if (conn_state_.connected && conn_state_.sock_fd >= 0) {
                uint8_t buffer[2048];
                uint32_t offset = 0;

                buffer[offset++] = 0x03;

                auto write_string = [&](const std::string& str) {
                    uint32_t len = str.size();
                    memcpy(buffer + offset, &len, sizeof(len));
                    offset += sizeof(len);
                    memcpy(buffer + offset, str.c_str(), len);
                    offset += len;
                };

                write_string(queued_error.error_type);
                write_string(queued_error.severity);
                write_string(queued_error.message);
                write_string(queued_error.source);

                memcpy(buffer + offset, &queued_error.line, sizeof(queued_error.line));
                offset += sizeof(queued_error.line);

                uint32_t frame_len = offset;
                ssize_t n = send(conn_state_.sock_fd, &frame_len, sizeof(frame_len), 0);
                if (n >= 0) {
                    send(conn_state_.sock_fd, buffer, offset, 0);
                    flushed_count++;
                }
            }
        }

        error_queue_.pop();
    }

    if (flushed_count > 0) {
        std::cout << "Flushed " << flushed_count << " queued error(s)" << std::endl;
    }
}

bool UdsClient::SendError(const DOMError& error) {
    std::lock_guard<std::mutex> lock(conn_mutex_);

    if (!conn_state_.connected || conn_state_.sock_fd < 0) {
        // Queue error for later delivery
        std::lock_guard<std::mutex> queue_lock(error_queue_mutex_);
        error_queue_.push(error);

        if (error_queue_.size() == 1) {
            std::cout << "Queuing error (not connected yet): " << error.message << std::endl;
        }
        return true;  // Return true since we queued it
    }

    uint8_t buffer[2048];
    uint32_t offset = 0;

    uint8_t msg_type = 0x03;
    buffer[offset++] = msg_type;

    auto write_string = [&](const std::string& str) {
        uint32_t len = str.size();
        memcpy(buffer + offset, &len, sizeof(len));
        offset += sizeof(len);
        memcpy(buffer + offset, str.c_str(), len);
        offset += len;
    };

    write_string(error.error_type);
    write_string(error.severity);
    write_string(error.message);
    write_string(error.source);

    memcpy(buffer + offset, &error.line, sizeof(error.line));
    offset += sizeof(error.line);

    uint32_t frame_len = offset;
    ssize_t n = send(conn_state_.sock_fd, &frame_len, sizeof(frame_len), 0);
    if (n < 0) {
        std::cerr << "Failed to send error frame length: " << strerror(errno) << std::endl;
        return false;
    }

    n = send(conn_state_.sock_fd, buffer, offset, 0);
    if (n < 0) {
        std::cerr << "Failed to send error: " << strerror(errno) << std::endl;
        return false;
    }

    return true;
}
