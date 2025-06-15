import Foundation
import XCTest

/// Generic AXP service that runs in background to provide fast touch injection
/// This doesn't need to know about any specific app - it just provides AXP capabilities
@objc public class ArkavoAXPService: NSObject {
    
    static let shared = ArkavoAXPService()
    private let socketPath = "{{SOCKET_PATH}}"
    private var localSocket: CFSocket?
    private let axBridge = ArkavoAXBridge()
    
    public func start() {
        print("[ArkavoAXPService] Starting AXP service...")
        print("[ArkavoAXPService] AXP available: \(axBridge.isAvailable())")
        
        // Start socket server
        startSocketServer()
        
        // Keep service running
        RunLoop.current.run()
    }
    
    private func startSocketServer() {
        // Remove existing socket
        try? FileManager.default.removeItem(atPath: socketPath)
        
        // Create Unix domain socket
        var context = CFSocketContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil,
            release: nil,
            copyDescription: nil
        )
        
        localSocket = CFSocketCreate(
            kCFAllocatorDefault,
            PF_UNIX,
            SOCK_STREAM,
            0,
            CFSocketCallBackType.acceptCallBack.rawValue,
            { (socket, callbackType, address, data, info) in
                guard let info = info else { return }
                let service = Unmanaged<ArkavoAXPService>.fromOpaque(info).takeUnretainedValue()
                service.handleConnection(data!)
            },
            &context
        )
        
        // Bind to socket path
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        socketPath.withCString { ptr in
            withUnsafeMutablePointer(to: &addr.sun_path) { pathPtr in
                _ = strcpy(pathPtr, ptr)
            }
        }
        
        let addressData = withUnsafePointer(to: &addr) { ptr in
            CFDataCreate(nil, UnsafeRawPointer(ptr).assumingMemoryBound(to: UInt8.self), MemoryLayout<sockaddr_un>.size)
        }
        
        CFSocketSetAddress(localSocket, addressData)
        
        // Add to run loop
        let source = CFSocketCreateRunLoopSource(kCFAllocatorDefault, localSocket, 0)
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .defaultMode)
        
        print("[ArkavoAXPService] Listening on: \(socketPath)")
    }
    
    private func handleConnection(_ data: UnsafeRawPointer) {
        let handle = CFSocketNativeHandle(bitPattern: data.load(as: Int.self))!
        
        DispatchQueue.global(qos: .userInteractive).async {
            self.processCommands(handle: handle)
        }
    }
    
    private func processCommands(handle: CFSocketNativeHandle) {
        let inputStream = InputStream(fileDescriptor: handle, retainReference: false)
        let outputStream = OutputStream(fileDescriptor: handle, retainReference: false)
        
        inputStream.open()
        outputStream.open()
        
        defer {
            inputStream.close()
            outputStream.close()
            close(handle)
        }
        
        // Send capabilities
        let capabilities = axBridge.capabilities()
        sendResponse(["type": "ready", "capabilities": capabilities], to: outputStream)
        
        let bufferSize = 4096
        let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: bufferSize)
        defer { buffer.deallocate() }
        
        var messageBuffer = ""
        
        while inputStream.hasBytesAvailable {
            let bytesRead = inputStream.read(buffer, maxLength: bufferSize)
            
            if bytesRead <= 0 { break }
            
            let data = String(bytesDecoding: Data(bytes: buffer, count: bytesRead), as: UTF8.self)
            messageBuffer += data
            
            // Process newline-delimited messages
            while let newlineIndex = messageBuffer.firstIndex(of: "\n") {
                let messageData = String(messageBuffer[..<newlineIndex])
                messageBuffer.removeSubrange(...newlineIndex)
                
                if let jsonData = messageData.data(using: .utf8),
                   let command = try? JSONSerialization.jsonObject(with: jsonData) as? [String: Any] {
                    
                    let response = axBridge.processCommand(command)
                    sendResponse(response, to: outputStream)
                }
            }
        }
    }
    
    private func sendResponse(_ response: [String: Any], to stream: OutputStream) {
        do {
            let jsonData = try JSONSerialization.data(withJSONObject: response)
            var message = String(data: jsonData, encoding: .utf8)! + "\n"
            message.withUTF8 { bytes in
                stream.write(bytes.baseAddress!, maxLength: bytes.count)
            }
        } catch {
            print("[ArkavoAXPService] Failed to send response: \(error)")
        }
    }
}