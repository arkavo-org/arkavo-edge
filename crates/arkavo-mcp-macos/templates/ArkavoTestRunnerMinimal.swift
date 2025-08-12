import Foundation

/// Minimal test runner for iOS 26 beta compatibility
/// This version works without XCTest framework
@objc public class ArkavoTestRunnerMinimal: NSObject {
    
    // MARK: - Properties
    
    static let socketPath = "{{SOCKET_PATH}}"
    private var socketServer: SocketServer?
    
    // MARK: - Setup
    
    @objc public override init() {
        super.init()
        print("[ArkavoTestRunnerMinimal] Initializing iOS 26 beta minimal runner")
    }
    
    @objc public class func setUp() {
        print("[ArkavoTestRunnerMinimal] Setting up minimal test runner...")
        print("[ArkavoTestRunnerMinimal] Socket path: \(socketPath)")
        
        let runner = ArkavoTestRunnerMinimal()
        runner.startSocketServer()
    }
    
    // MARK: - Socket Server
    
    private func startSocketServer() {
        socketServer = SocketServer(socketPath: Self.socketPath)
        socketServer?.start()
    }
}

/// Simple socket server that doesn't depend on XCTest
class SocketServer {
    private let socketPath: String
    private var socket: Int32 = -1
    private var shouldStop = false
    
    init(socketPath: String) {
        self.socketPath = socketPath
    }
    
    func start() {
        // Remove existing socket
        unlink(socketPath)
        
        // Create socket
        socket = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard socket >= 0 else {
            print("[SocketServer] Failed to create socket")
            return
        }
        
        // Bind socket
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = socketPath.utf8CString
        withUnsafeMutablePointer(to: &addr.sun_path.0) { ptr in
            pathBytes.withUnsafeBufferPointer { buffer in
                ptr.initialize(from: buffer.baseAddress!, count: min(buffer.count, 104))
            }
        }
        
        let bindResult = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                bind(socket, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        
        guard bindResult == 0 else {
            print("[SocketServer] Failed to bind socket: \(String(cString: strerror(errno)))")
            close(socket)
            return
        }
        
        // Listen
        guard listen(socket, 5) == 0 else {
            print("[SocketServer] Failed to listen: \(String(cString: strerror(errno)))")
            close(socket)
            return
        }
        
        print("[SocketServer] Listening on \(socketPath)")
        
        // Send ready message
        DispatchQueue.global(qos: .background).asyncAfter(deadline: .now() + 0.5) {
            self.acceptConnections()
        }
    }
    
    private func acceptConnections() {
        while !shouldStop {
            var addr = sockaddr_un()
            var addrLen = socklen_t(MemoryLayout<sockaddr_un>.size)
            
            let clientSocket = withUnsafeMutablePointer(to: &addr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                    accept(socket, sockaddrPtr, &addrLen)
                }
            }
            
            guard clientSocket >= 0 else {
                if errno != EINTR {
                    print("[SocketServer] Accept failed: \(String(cString: strerror(errno)))")
                }
                continue
            }
            
            print("[SocketServer] Client connected")
            handleClient(clientSocket)
        }
    }
    
    private func handleClient(_ clientSocket: Int32) {
        // Send ready message
        let readyMsg = """
        {"type":"ready","capabilities":{"mode":"minimal","ios26_beta":true,"note":"Using minimal bridge for iOS 26 beta compatibility"}}
        """
        _ = send(clientSocket, readyMsg, readyMsg.count, 0)
        
        // Read loop
        let bufferSize = 1024
        let buffer = UnsafeMutablePointer<CChar>.allocate(capacity: bufferSize)
        defer { buffer.deallocate() }
        
        var messageBuffer = ""
        
        while true {
            let bytesRead = recv(clientSocket, buffer, bufferSize - 1, 0)
            
            if bytesRead <= 0 {
                break
            }
            
            buffer[bytesRead] = 0
            let chunk = String(cString: buffer)
            messageBuffer += chunk
            
            // Process complete messages (newline delimited)
            while let newlineRange = messageBuffer.range(of: "\n") {
                let message = String(messageBuffer[..<newlineRange.lowerBound])
                messageBuffer.removeSubrange(...newlineRange.lowerBound)
                
                if let response = processMessage(message) {
                    let responseStr = response + "\n"
                    _ = send(clientSocket, responseStr, responseStr.count, 0)
                }
            }
        }
        
        close(clientSocket)
        print("[SocketServer] Client disconnected")
    }
    
    private func processMessage(_ message: String) -> String? {
        guard let data = message.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return "{\"error\":\"Invalid JSON\"}"
        }
        
        // Basic message handling
        let response: [String: Any] = [
            "id": json["id"] ?? "",
            "success": false,
            "error": "iOS 26 beta minimal mode - operations not supported",
            "note": "Use IDB or AppleScript for actual automation"
        ]
        
        if let responseData = try? JSONSerialization.data(withJSONObject: response),
           let responseStr = String(data: responseData, encoding: .utf8) {
            return responseStr
        }
        
        return "{\"error\":\"Failed to encode response\"}"
    }
    
    func stop() {
        shouldStop = true
        close(socket)
    }
}