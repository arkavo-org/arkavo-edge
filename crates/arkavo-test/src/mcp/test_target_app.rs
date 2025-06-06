use crate::{Result, TestError};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Creates a simple test target app for XCTest to interact with
pub struct TestTargetApp {
    build_dir: PathBuf,
    app_name: String,
}

impl TestTargetApp {
    pub fn new() -> Result<Self> {
        let build_dir = std::env::temp_dir().join("arkavo-test-target-app");
        fs::create_dir_all(&build_dir)?;
        
        Ok(Self {
            build_dir,
            app_name: "ArkavoTestTarget".to_string(),
        })
    }
    
    pub fn app_bundle_id(&self) -> String {
        "com.arkavo.testtarget".to_string()
    }
    
    pub fn app_path(&self) -> PathBuf {
        self.build_dir.join(format!("{}.app", self.app_name))
    }
    
    /// Build and install a simple test app with buttons and text fields
    pub fn build_and_install(&self, device_id: &str) -> Result<()> {
        eprintln!("[TestTargetApp] Building test target app...");
        
        let app_dir = self.app_path();
        fs::create_dir_all(&app_dir)?;
        
        // Create Info.plist
        let info_plist = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>{}</string>
    <key>CFBundleIdentifier</key>
    <string>{}</string>
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
    <key>UILaunchStoryboardName</key>
    <string>LaunchScreen</string>
</dict>
</plist>"#, self.app_name, self.app_bundle_id(), self.app_name);
        
        fs::write(app_dir.join("Info.plist"), info_plist)?;
        
        // Create Swift source with a simple UI
        let swift_source = r#"
import UIKit

class ViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        
        // Create a stack view
        let stackView = UIStackView()
        stackView.axis = .vertical
        stackView.spacing = 20
        stackView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stackView)
        
        // Title label
        let titleLabel = UILabel()
        titleLabel.text = "Arkavo Test Target"
        titleLabel.font = .systemFont(ofSize: 24, weight: .bold)
        titleLabel.textAlignment = .center
        titleLabel.accessibilityIdentifier = "titleLabel"
        stackView.addArrangedSubview(titleLabel)
        
        // Text field
        let textField = UITextField()
        textField.placeholder = "Enter text here"
        textField.borderStyle = .roundedRect
        textField.accessibilityIdentifier = "inputField"
        stackView.addArrangedSubview(textField)
        
        // Buttons
        let button1 = UIButton(type: .system)
        button1.setTitle("Tap Me", for: .normal)
        button1.accessibilityIdentifier = "tapButton"
        button1.addTarget(self, action: #selector(buttonTapped(_:)), for: .touchUpInside)
        stackView.addArrangedSubview(button1)
        
        let button2 = UIButton(type: .system)
        button2.setTitle("Continue", for: .normal)
        button2.accessibilityIdentifier = "continueButton"
        button2.addTarget(self, action: #selector(buttonTapped(_:)), for: .touchUpInside)
        stackView.addArrangedSubview(button2)
        
        let button3 = UIButton(type: .system)
        button3.setTitle("Save", for: .normal)
        button3.accessibilityIdentifier = "saveButton"
        button3.addTarget(self, action: #selector(buttonTapped(_:)), for: .touchUpInside)
        stackView.addArrangedSubview(button3)
        
        // Result label
        let resultLabel = UILabel()
        resultLabel.text = "Ready"
        resultLabel.textAlignment = .center
        resultLabel.accessibilityIdentifier = "resultLabel"
        stackView.addArrangedSubview(resultLabel)
        
        // Constraints
        NSLayoutConstraint.activate([
            stackView.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stackView.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            stackView.leadingAnchor.constraint(greaterThanOrEqualTo: view.leadingAnchor, constant: 20),
            stackView.trailingAnchor.constraint(lessThanOrEqualTo: view.trailingAnchor, constant: -20)
        ])
    }
    
    @objc func buttonTapped(_ sender: UIButton) {
        print("Button tapped: \(sender.currentTitle ?? "Unknown")")
        
        // Update result label
        if let resultLabel = view.subviews
            .compactMap({ $0 as? UIStackView }).first?
            .arrangedSubviews
            .compactMap({ $0 as? UILabel })
            .first(where: { $0.accessibilityIdentifier == "resultLabel" }) {
            resultLabel.text = "Tapped: \(sender.currentTitle ?? "Unknown")"
        }
    }
}

class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?
    
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = ViewController()
        window?.makeKeyAndVisible()
        return true
    }
}

UIApplicationMain(
    CommandLine.argc,
    CommandLine.unsafeArgv,
    nil,
    NSStringFromClass(AppDelegate.self)
)
"#;
        
        // Write source
        let source_path = self.build_dir.join("main.swift");
        fs::write(&source_path, swift_source)?;
        
        // Compile
        let binary_path = app_dir.join(&self.app_name);
        
        let sdk_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
            .output()?;
        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout).trim().to_string();
        
        let compile_output = Command::new("xcrun")
            .args([
                "swiftc",
                "-sdk", &sdk_path,
                "-target", "arm64-apple-ios15.0-simulator",
                "-framework", "UIKit",
                "-o", binary_path.to_str().unwrap(),
                source_path.to_str().unwrap(),
            ])
            .output()?;
            
        if !compile_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to compile test app: {}",
                String::from_utf8_lossy(&compile_output.stderr)
            )));
        }
        
        // Sign
        let _ = Command::new("codesign")
            .args([
                "--force",
                "--sign", "-",
                app_dir.to_str().unwrap()
            ])
            .output();
        
        // Install
        eprintln!("[TestTargetApp] Installing test target app...");
        
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
                "Failed to install test app: {}",
                String::from_utf8_lossy(&install_output.stderr)
            )));
        }
        
        eprintln!("[TestTargetApp] Test target app installed successfully");
        Ok(())
    }
}