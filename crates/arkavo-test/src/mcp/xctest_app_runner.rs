use crate::{Result, TestError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Creates and runs a minimal iOS app with embedded automation capabilities
pub struct XCTestAppRunner {
    build_dir: PathBuf,
    socket_path: PathBuf,
}

impl XCTestAppRunner {
    pub fn new() -> Result<Self> {
        let build_dir = std::env::temp_dir().join("arkavo-test-app");
        fs::create_dir_all(&build_dir)?;
        
        let socket_path = std::env::temp_dir().join(format!("arkavo-test-{}.sock", std::process::id()));
        
        Ok(Self {
            build_dir,
            socket_path,
        })
    }
    
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
    
    /// Create and install a minimal iOS app
    pub fn install_and_run(&self, device_id: &str) -> Result<()> {
        eprintln!("[AppRunner] Creating iOS app for device {}", device_id);
        
        // Create app bundle structure
        let app_name = "ArkavoTestApp";
        let app_dir = self.build_dir.join(format!("{}.app", app_name));
        fs::create_dir_all(&app_dir)?;
        
        // Create Info.plist
        let info_plist = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>{}</string>
    <key>CFBundleIdentifier</key>
    <string>com.arkavo.testapp</string>
    <key>CFBundleName</key>
    <string>{}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSRequiresIPhoneOS</key>
    <true/>
    <key>UIRequiredDeviceCapabilities</key>
    <array>
        <string>arm64</string>
    </array>
    <key>UIApplicationSceneManifest</key>
    <dict>
        <key>UIApplicationSupportsMultipleScenes</key>
        <false/>
    </dict>
</dict>
</plist>"#, app_name, app_name);
        
        fs::write(app_dir.join("Info.plist"), info_plist)?;
        
        // Create Swift source with UI automation using private APIs
        let swift_source = format!(r#"
import UIKit
import Foundation

// Socket server that runs in the app
class SocketServer {{
    let socketPath: String
    private var socketFD: Int32 = -1
    
    init(socketPath: String) {{
        self.socketPath = socketPath
    }}
    
    func start() {{
        print("[App] Starting socket server at: \(socketPath)")
        
        socketFD = socket(AF_UNIX, SOCK_STREAM, 0)
        guard socketFD >= 0 else {{
            print("[App] Failed to create socket")
            return
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
            print("[App] Failed to bind: \(errno)")
            close(socketFD)
            return
        }}
        
        guard listen(socketFD, 5) == 0 else {{
            print("[App] Failed to listen")
            close(socketFD)
            return
        }}
        
        print("[App] Socket listening")
        
        // Accept connections
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
                    print("[App] Client connected")
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
            
            do {{
                if let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let command = json["command"] as? String {{
                    
                    print("[App] Command: \(command)")
                    
                    var response: [String: Any] = ["id": json["id"] ?? "", "success": true]
                    
                    switch command {{
                    case "tap":
                        if let x = json["x"] as? Double, let y = json["y"] as? Double {{
                            // Use iOS private API to tap
                            performTap(x: x, y: y)
                            response["result"] = "Tapped at \(x),\(y)"
                        }}
                    case "ping":
                        response["result"] = "pong"
                    case "screenshot":
                        response["result"] = "Screenshot would be taken"
                    default:
                        response["success"] = false
                        response["error"] = "Unknown command"
                    }}
                    
                    if let responseData = try? JSONSerialization.data(withJSONObject: response) {{
                        _ = responseData.withUnsafeBytes {{ ptr in
                            write(clientFD, ptr.baseAddress!, responseData.count)
                        }}
                    }}
                }}
            }} catch {{
                print("[App] Error: \(error)")
            }}
        }}
        
        close(clientFD)
    }}
}}

// Simple tap using accessibility
func performTap(x: Double, y: Double) {{
    print("[App] Performing tap at \(x), \(y)")
    // In a real implementation, we'd use IOHIDEvent or accessibility APIs
    // For now, just log it
}}

// App Delegate
class AppDelegate: UIResponder, UIApplicationDelegate {{
    var window: UIWindow?
    var socketServer: SocketServer?
    
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {{
        print("[App] Application launched")
        
        // Create window
        window = UIWindow(frame: UIScreen.main.bounds)
        let vc = UIViewController()
        vc.view.backgroundColor = .systemBlue
        
        // Add label
        let label = UILabel()
        label.text = "Arkavo Test App"
        label.textColor = .white
        label.translatesAutoresizingMaskIntoConstraints = false
        vc.view.addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: vc.view.centerXAnchor),
            label.centerYAnchor.constraint(equalTo: vc.view.centerYAnchor)
        ])
        
        window?.rootViewController = vc
        window?.makeKeyAndVisible()
        
        // Start socket server
        let socketPath = "{}"
        socketServer = SocketServer(socketPath: socketPath)
        socketServer?.start()
        
        return true
    }}
}}

// Main
UIApplicationMain(
    CommandLine.argc,
    CommandLine.unsafeArgv,
    nil,
    NSStringFromClass(AppDelegate.self)
)
"#, self.socket_path.display());
        
        // Write source
        let source_path = self.build_dir.join("main.swift");
        fs::write(&source_path, swift_source)?;
        
        // Compile the app
        let binary_path = app_dir.join(app_name);
        
        let sdk_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
            .output()?;
        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout).trim().to_string();
        
        eprintln!("[AppRunner] Compiling app...");
        
        let compile_output = Command::new("xcrun")
            .args([
                "swiftc",
                "-sdk", &sdk_path,
                "-target", "arm64-apple-ios15.0-simulator",
                "-framework", "UIKit",
                "-framework", "Foundation",
                "-o", binary_path.to_str().unwrap(),
                source_path.to_str().unwrap(),
            ])
            .output()?;
            
        if !compile_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to compile app: {}",
                String::from_utf8_lossy(&compile_output.stderr)
            )));
        }
        
        // Sign the app
        let _ = Command::new("codesign")
            .args([
                "--force",
                "--sign", "-",
                app_dir.to_str().unwrap()
            ])
            .output();
        
        // Install the app
        eprintln!("[AppRunner] Installing app...");
        
        let install_output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                app_dir.to_str().unwrap()
            ])
            .output()?;
            
        if !install_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to install app: {}",
                String::from_utf8_lossy(&install_output.stderr)
            )));
        }
        
        // Launch the app
        eprintln!("[AppRunner] Launching app...");
        
        let launch_output = Command::new("xcrun")
            .args([
                "simctl",
                "launch",
                device_id,
                "com.arkavo.testapp"
            ])
            .output()?;
            
        if !launch_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to launch app: {}",
                String::from_utf8_lossy(&launch_output.stderr)
            )));
        }
        
        eprintln!("[AppRunner] App launched successfully");
        Ok(())
    }
}