use crate::{Result, TestError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Direct XCTest runner - runs Swift code with XCTest as a regular executable
pub struct XCTestDirectRunner {
    build_dir: PathBuf,
    socket_path: PathBuf,
}

impl XCTestDirectRunner {
    pub fn new() -> Result<Self> {
        let build_dir = std::env::temp_dir().join("arkavo-direct-xctest");
        fs::create_dir_all(&build_dir)?;

        let socket_path =
            std::env::temp_dir().join(format!("arkavo-xctest-{}.sock", std::process::id()));

        Ok(Self {
            build_dir,
            socket_path,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Compile and run XCTest code directly
    pub fn run_on_simulator(&self, device_id: &str) -> Result<()> {
        eprintln!(
            "[DirectRunner] Creating XCTest executable for device {}",
            device_id
        );

        // Create Swift code that uses XCTest but runs as a regular executable
        let swift_source = format!(
            r#"
import Foundation
import XCTest

// Make the socket server without namespace issues
class UnixSocketServer: NSObject {{
    let socketPath: String
    private var socketFD: Int32 = -1
    private var commandHandler: ((Data) -> Void)?
    
    init(socketPath: String) {{
        self.socketPath = socketPath
        super.init()
    }}
    
    func start(commandHandler: @escaping (Data) -> Void) throws {{
        self.commandHandler = commandHandler
        
        print("[Swift] Starting socket server at: \(socketPath)")
        
        socketFD = socket(AF_UNIX, SOCK_STREAM, 0)
        guard socketFD >= 0 else {{
            throw NSError(domain: "Socket", code: -1, userInfo: [NSLocalizedDescriptionKey: "Failed to create socket"])
        }}
        
        unlink(socketPath)
        
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        socketPath.withCString {{ ptr in
            withUnsafeMutablePointer(to: &addr.sun_path.0) {{ dst in
                strcpy(dst, ptr)
            }}
        }}
        
        let bindResult = withUnsafePointer(to: &addr) {{ ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) {{ sockaddrPtr in
                bind(socketFD, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_un>.size))
            }}
        }}
        
        guard bindResult == 0 else {{
            close(socketFD)
            throw NSError(domain: "Socket", code: -2, userInfo: [NSLocalizedDescriptionKey: "Failed to bind"])
        }}
        
        guard listen(socketFD, 5) == 0 else {{
            close(socketFD)
            throw NSError(domain: "Socket", code: -3, userInfo: [NSLocalizedDescriptionKey: "Failed to listen"])
        }}
        
        print("[Swift] Socket server listening")
        
        // Accept connections in background
        DispatchQueue.global().async {{
            while true {{
                var clientAddr = sockaddr_un()
                var clientAddrLen = socklen_t(MemoryLayout<sockaddr_un>.size)
                
                let clientFD = withUnsafeMutablePointer(to: &clientAddr) {{ ptr in
                    ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) {{ sockaddrPtr in
                        accept(self.socketFD, sockaddrPtr, &clientAddrLen)
                    }}
                }}
                
                if clientFD >= 0 {{
                    print("[Swift] Client connected")
                    self.handleClient(clientFD)
                }}
            }}
        }}
    }}
    
    private func handleClient(_ clientFD: Int32) {{
        var buffer = [UInt8](repeating: 0, count: 4096)
        
        while true {{
            let bytesRead = read(clientFD, &buffer, buffer.count)
            if bytesRead <= 0 {{ break }}
            
            let data = Data(bytes: buffer, count: Int(bytesRead))
            
            // Parse JSON command
            do {{
                if let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let command = json["command"] as? String {{
                    
                    print("[Swift] Received command: \(command)")
                    
                    var response: [String: Any] = ["id": json["id"] ?? "", "success": true]
                    
                    switch command {{
                    case "tap":
                        if let x = json["x"] as? Double, let y = json["y"] as? Double {{
                            // For now, just acknowledge the tap without using XCTest
                            // In a real implementation, we'd need to be in a proper XCTest context
                            response["result"] = "Would tap at \(x),\(y)"
                            print("[Swift] Tap command received for \(x),\(y)")
                        }}
                    case "ping":
                        response["result"] = "pong"
                    case "echo":
                        response["result"] = json["message"] ?? "no message"
                    default:
                        response["success"] = false
                        response["error"] = "Unknown command: \(command)"
                    }}
                    
                    // Send response
                    if let responseData = try? JSONSerialization.data(withJSONObject: response) {{
                        _ = responseData.withUnsafeBytes {{ ptr in
                            write(clientFD, ptr.baseAddress!, responseData.count)
                        }}
                    }}
                }}
            }} catch {{
                print("[Swift] Error parsing command: \(error)")
            }}
        }}
        
        close(clientFD)
    }}
}}

// Main entry point
print("[Swift] Starting XCTest automation server")
let socketPath = "{}"

// Start socket server first, without XCTest initialization
let server = UnixSocketServer(socketPath: socketPath)
do {{
    try server.start {{ data in
        print("[Swift] Received data: \(data)")
    }}
    
    print("[Swift] Server started, ready for commands")
    
    // Keep running
    RunLoop.current.run()
}} catch {{
    print("[Swift] Failed to start server: \(error)")
    exit(1)
}}
"#,
            self.socket_path.display()
        );

        // Write source
        let source_path = self.build_dir.join("xctest_direct.swift");
        fs::write(&source_path, swift_source)?;

        // Compile with XCTest framework
        let binary_path = self.build_dir.join("xctest_direct");

        let sdk_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
            .output()?;
        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout)
            .trim()
            .to_string();

        let platform_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-platform-path"])
            .output()?;
        let platform_path = String::from_utf8_lossy(&platform_output.stdout)
            .trim()
            .to_string();
        let xctest_framework_path = format!("{}/Developer/Library/Frameworks", platform_path);

        eprintln!("[DirectRunner] Compiling with XCTest framework...");

        let compile_output = Command::new("xcrun")
            .args([
                "swiftc",
                "-sdk",
                &sdk_path,
                "-target",
                "arm64-apple-ios15.0-simulator",
                "-F",
                &xctest_framework_path,
                "-framework",
                "XCTest",
                "-framework",
                "UIKit",
                "-framework",
                "Foundation",
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                &xctest_framework_path,
                "-o",
                binary_path.to_str().unwrap(),
                source_path.to_str().unwrap(),
            ])
            .output()?;

        if !compile_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to compile: {}",
                String::from_utf8_lossy(&compile_output.stderr)
            )));
        }

        eprintln!("[DirectRunner] Compiled successfully, spawning on simulator...");

        // Run on simulator
        let mut child = Command::new("xcrun")
            .args(["simctl", "spawn", device_id, binary_path.to_str().unwrap()])
            .spawn()
            .map_err(|e| TestError::Mcp(format!("Failed to spawn: {}", e)))?;

        // Give it time to start
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Check if running
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    // Get output
                    let output = child.wait_with_output()?;
                    return Err(TestError::Mcp(format!(
                        "Process exited with: {}. Output: {}",
                        status,
                        String::from_utf8_lossy(&output.stderr)
                    )));
                }
            }
            Ok(None) => {
                eprintln!("[DirectRunner] XCTest server running");
            }
            Err(e) => {
                eprintln!("[DirectRunner] Warning: Could not check status: {}", e);
            }
        }

        Ok(())
    }
}
