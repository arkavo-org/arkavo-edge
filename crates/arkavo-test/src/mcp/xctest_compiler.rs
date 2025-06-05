use super::templates;
use crate::{Result, TestError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static XCTEST_BUNDLE_CACHE: OnceLock<PathBuf> = OnceLock::new();

pub struct XCTestCompiler {
    build_dir: PathBuf,
    socket_path: PathBuf,
}

impl XCTestCompiler {
    pub fn new() -> Result<Self> {
        // Check if Xcode is available
        let xcode_check = Command::new("xcrun")
            .args(["--version"])
            .output()
            .map_err(|e| TestError::Mcp(format!(
                "xcrun not found. Xcode or Xcode Command Line Tools must be installed.\n\
                Install Xcode from the App Store or run: xcode-select --install\n\
                Error: {}", e
            )))?;
            
        if !xcode_check.status.success() {
            return Err(TestError::Mcp(
                "xcrun failed. Make sure Xcode Command Line Tools are properly configured.\n\
                Run: sudo xcode-select --switch /Applications/Xcode.app\n\
                Or: xcode-select --install".to_string()
            ));
        }
        
        let build_dir = std::env::temp_dir().join("arkavo-xctest-build");
        eprintln!("[XCTestCompiler] Build directory: {}", build_dir.display());
        eprintln!("[XCTestCompiler] Using embedded templates (compiled into binary)");

        // Create build directory if it doesn't exist
        fs::create_dir_all(&build_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create build directory: {}", e)))?;

        // Generate socket path
        let socket_path =
            std::env::temp_dir().join(format!("arkavo-xctest-{}.sock", std::process::id()));
        eprintln!("[XCTestCompiler] Socket path: {}", socket_path.display());

        Ok(Self {
            build_dir,
            socket_path,
        })
    }
    

    /// Get the socket path for communication
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Get or compile the XCTest bundle
    pub fn get_xctest_bundle(&self) -> Result<PathBuf> {
        // Check cache first
        if let Some(cached_path) = XCTEST_BUNDLE_CACHE.get() {
            if cached_path.exists() {
                eprintln!("Using cached XCTest bundle at: {}", cached_path.display());
                return Ok(cached_path.clone());
            }
        }

        // Compile new bundle
        let bundle_path = self.compile_xctest_bundle()?;

        // Cache the result
        let _ = XCTEST_BUNDLE_CACHE.set(bundle_path.clone());

        Ok(bundle_path)
    }

    /// Compile the XCTest bundle from templates
    fn compile_xctest_bundle(&self) -> Result<PathBuf> {
        eprintln!("[XCTestCompiler] Starting XCTest bundle compilation...");

        // Step 1: Create temporary source directory
        let source_dir = self.build_dir.join("Sources");
        fs::create_dir_all(&source_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create source directory: {}", e)))?;

        // Step 2: Process and copy templates
        self.process_templates(&source_dir)?;

        // Step 3: Create minimal Package.swift for compilation
        self.create_package_swift(&self.build_dir)?;

        // Step 4: Compile using swift build
        self.compile_swift_package(&self.build_dir)?;

        // Step 5: Create .xctest bundle structure
        let xctest_bundle = self.create_xctest_bundle(&self.build_dir)?;

        eprintln!(
            "XCTest bundle compiled successfully at: {}",
            xctest_bundle.display()
        );

        Ok(xctest_bundle)
    }

    /// Process templates and write to source directory
    fn process_templates(&self, source_dir: &Path) -> Result<()> {
        eprintln!("[XCTestCompiler] Using embedded Swift template from binary");
        
        // Use the embedded template - always use the basic one that we know works
        let swift_template = templates::ARKAVO_TEST_RUNNER_SWIFT;
        
        // The verification is now done at compile time via tests, but let's double-check
        debug_assert!(!swift_template.contains("let result: [String: Any]?"), 
            "Embedded template should not contain [String: Any]");
        debug_assert!(swift_template.contains("enum JSONValue: Codable"),
            "Embedded template should contain JSONValue enum");

        // Replace template variables
        let swift_source =
            swift_template.replace("{{SOCKET_PATH}}", &self.socket_path.to_string_lossy());

        // Write Swift source
        let swift_path = source_dir.join("ArkavoTestRunner.swift");
        fs::write(&swift_path, swift_source)
            .map_err(|e| TestError::Mcp(format!("Failed to write Swift source: {}", e)))?;

        // Use embedded Info.plist template
        eprintln!("[XCTestCompiler] Using embedded Info.plist template from binary");
        let plist_content = templates::INFO_PLIST;

        let plist_path = self.build_dir.join("Info.plist");
        fs::write(&plist_path, plist_content)
            .map_err(|e| TestError::Mcp(format!("Failed to write Info.plist: {}", e)))?;

        Ok(())
    }

    /// Create Package.swift for Swift Package Manager
    fn create_package_swift(&self, build_dir: &Path) -> Result<()> {
        let package_swift = r#"// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "ArkavoTestRunner",
    platforms: [
        .iOS(.v15)
    ],
    products: [
        .library(
            name: "ArkavoTestRunner",
            type: .dynamic,
            targets: ["ArkavoTestRunner"]
        )
    ],
    targets: [
        .target(
            name: "ArkavoTestRunner",
            dependencies: [],
            path: "Sources"
        )
    ]
)
"#;

        let package_path = build_dir.join("Package.swift");
        fs::write(&package_path, package_swift)
            .map_err(|e| TestError::Mcp(format!("Failed to write Package.swift: {}", e)))?;

        Ok(())
    }

    /// Compile the Swift package
    fn compile_swift_package(&self, build_dir: &Path) -> Result<()> {
        eprintln!("[XCTestCompiler] Compiling Swift package...");
        
        // For XCTest bundles on simulator, we don't need code signing
        // Use direct swift compilation instead of xcodebuild
        let swift_source = build_dir.join("Sources/ArkavoTestRunner.swift");
        let output_binary = build_dir.join("ArkavoTestRunner");
        
        // Dynamically get iOS SDK path using xcrun (works on any machine with Xcode)
        let sdk_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get SDK path: {}\nMake sure Xcode is installed and command line tools are configured.\nRun: xcode-select --install", e)))?;
            
        if !sdk_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to get iOS SDK path. Make sure Xcode is installed.\nError: {}",
                String::from_utf8_lossy(&sdk_output.stderr)
            )));
        }
            
        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout).trim().to_string();
        eprintln!("[XCTestCompiler] Using SDK: {}", sdk_path);
        
        // Dynamically get platform path for frameworks
        let platform_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-platform-path"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get platform path: {}", e)))?;
            
        if !platform_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to get platform path.\nError: {}",
                String::from_utf8_lossy(&platform_output.stderr)
            )));
        }
            
        let platform_path = String::from_utf8_lossy(&platform_output.stdout).trim().to_string();
        let xctest_framework_path = format!("{}/Developer/Library/Frameworks", platform_path);
        eprintln!("[XCTestCompiler] XCTest framework path: {}", xctest_framework_path);
        
        // Verify XCTest framework exists
        if !std::path::Path::new(&format!("{}/XCTest.framework", xctest_framework_path)).exists() {
            return Err(TestError::Mcp(format!(
                "XCTest.framework not found at {}. Xcode may not be properly installed.",
                xctest_framework_path
            )));
        }
        
        // Compile as a framework/bundle
        let output = Command::new("xcrun")
            .args([
                "swiftc",
                "-sdk", &sdk_path,
                "-target", "x86_64-apple-ios15.0-simulator",
                "-emit-library",
                "-emit-module",
                "-module-name", "ArkavoTestRunner",
                "-Xlinker", "-bundle",
                "-Xlinker", "-rpath",
                "-Xlinker", "@executable_path/Frameworks",
                "-Xlinker", "-rpath",
                "-Xlinker", "@loader_path/Frameworks",
                "-F", &xctest_framework_path,
                "-F", &sdk_path,
                "-framework", "XCTest",
                "-o", output_binary.to_str().unwrap(),
                swift_source.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run swift compiler: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            eprintln!("[XCTestCompiler] Compilation failed");
            eprintln!("[XCTestCompiler] STDOUT: {}", stdout);
            eprintln!("[XCTestCompiler] STDERR: {}", stderr);
            
            // Try arm64 target if x86_64 failed
            eprintln!("[XCTestCompiler] Trying arm64 target...");
            let output_arm = Command::new("xcrun")
                .args([
                    "swiftc",
                    "-sdk", &sdk_path,
                    "-target", "arm64-apple-ios15.0-simulator",
                    "-emit-library",
                    "-emit-module",
                    "-module-name", "ArkavoTestRunner",
                    "-Xlinker", "-bundle",
                    "-F", &xctest_framework_path,
                    "-F", &sdk_path,
                    "-framework", "XCTest",
                    "-o", output_binary.to_str().unwrap(),
                    swift_source.to_str().unwrap(),
                ])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to run swift compiler (arm64): {}", e)))?;
                
            if !output_arm.status.success() {
                let arm_stderr = String::from_utf8_lossy(&output_arm.stderr);
                return Err(TestError::Mcp(format!(
                    "Compilation failed for both architectures.\nx86_64 error: {}\narm64 error: {}",
                    stderr, arm_stderr
                )));
            }
        }

        Ok(())
    }

    /// Create the .xctest bundle structure
    fn create_xctest_bundle(&self, build_dir: &Path) -> Result<PathBuf> {
        let bundle_name = "ArkavoTestRunner.xctest";
        let bundle_path = build_dir.join(bundle_name);

        eprintln!("[XCTestCompiler] Creating bundle at: {}", bundle_path.display());

        // Create bundle directory structure
        fs::create_dir_all(&bundle_path)
            .map_err(|e| TestError::Mcp(format!("Failed to create bundle directory: {}", e)))?;

        // Copy Info.plist
        let plist_src = build_dir.join("Info.plist");
        let plist_dst = bundle_path.join("Info.plist");
        fs::copy(&plist_src, &plist_dst)
            .map_err(|e| TestError::Mcp(format!("Failed to copy Info.plist: {}", e)))?;

        // Find and copy the compiled binary
        let binary_src = build_dir.join("ArkavoTestRunner");
        let binary_dst = bundle_path.join("ArkavoTestRunner");
        
        if binary_src.exists() {
            eprintln!("[XCTestCompiler] Copying binary from {} to {}", binary_src.display(), binary_dst.display());
            fs::copy(&binary_src, &binary_dst)
                .map_err(|e| TestError::Mcp(format!("Failed to copy binary: {}", e)))?;

            // Make it executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&binary_dst)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&binary_dst, perms)?;
            }
            
            eprintln!("[XCTestCompiler] Binary copied successfully");
        } else {
            // Try DerivedData location as fallback
            let derived_data = build_dir.join("DerivedData");
            if derived_data.exists() {
                if let Ok(binary_path) = find_compiled_binary(&derived_data, "ArkavoTestRunner") {
                    eprintln!("[XCTestCompiler] Found binary in DerivedData at {}", binary_path.display());
                    fs::copy(&binary_path, &binary_dst)
                        .map_err(|e| TestError::Mcp(format!("Failed to copy binary from DerivedData: {}", e)))?;

                    // Make it executable
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut perms = fs::metadata(&binary_dst)?.permissions();
                        perms.set_mode(0o755);
                        fs::set_permissions(&binary_dst, perms)?;
                    }
                } else {
                    return Err(TestError::Mcp("Compiled binary not found in build directory or DerivedData".to_string()));
                }
            } else {
                return Err(TestError::Mcp(format!(
                    "Compiled binary not found at {}. Build may have failed.",
                    binary_src.display()
                )));
            }
        }

        Ok(bundle_path)
    }

    /// Install the XCTest bundle to a simulator
    pub fn install_to_simulator(&self, device_id: &str, bundle_path: &Path) -> Result<()> {
        eprintln!("[XCTestCompiler] Installing XCTest bundle to simulator {}...", device_id);
        
        // For XCTest bundles, we need to copy them to the simulator's app support directory
        // instead of using simctl install which is for regular apps
        
        // Get simulator data path
        let home = std::env::var("HOME").map_err(|_| TestError::Mcp("HOME not set".to_string()))?;
        let sim_data_path = PathBuf::from(&home)
            .join("Library/Developer/CoreSimulator/Devices")
            .join(device_id)
            .join("data");
            
        if !sim_data_path.exists() {
            return Err(TestError::Mcp(format!(
                "Simulator data directory not found: {}. Is the simulator created?",
                sim_data_path.display()
            )));
        }
        
        // XCTest bundles go in the Library/Developer/Xcode/DerivedData/TestBundles directory
        let test_bundles_dir = sim_data_path
            .join("Library/Developer/Xcode/DerivedData/TestBundles");
            
        // Create the directory if it doesn't exist
        fs::create_dir_all(&test_bundles_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create test bundles directory: {}", e)))?;
            
        // Copy the bundle
        let dest_path = test_bundles_dir.join(bundle_path.file_name().unwrap());
        
        // Remove existing bundle if present
        if dest_path.exists() {
            fs::remove_dir_all(&dest_path)
                .map_err(|e| TestError::Mcp(format!("Failed to remove existing bundle: {}", e)))?;
        }
        
        eprintln!("[XCTestCompiler] Copying bundle to {}", dest_path.display());
        
        // Use cp -R to preserve bundle structure
        let output = Command::new("cp")
            .args(["-R", bundle_path.to_str().unwrap(), dest_path.parent().unwrap().to_str().unwrap()])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to copy bundle: {}", e)))?;
            
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TestError::Mcp(format!(
                "Failed to copy test bundle: {}",
                stderr
            )));
        }
        
        eprintln!("[XCTestCompiler] XCTest bundle installed successfully");
        Ok(())
    }

    /// Run the XCTest bundle on a simulator
    pub fn run_tests(&self, device_id: &str, _bundle_id: &str) -> Result<()> {
        eprintln!("[XCTestCompiler] Running tests on device: {}", device_id);
        eprintln!("[XCTestCompiler] Socket path: {}", self.socket_path.display());
        
        // Get the path where we installed the bundle
        let home = std::env::var("HOME").map_err(|_| TestError::Mcp("HOME not set".to_string()))?;
        let bundle_path = PathBuf::from(&home)
            .join("Library/Developer/CoreSimulator/Devices")
            .join(device_id)
            .join("data/Library/Developer/Xcode/DerivedData/TestBundles/ArkavoTestRunner.xctest");
            
        if !bundle_path.exists() {
            return Err(TestError::Mcp(format!(
                "Test bundle not found at {}. Installation may have failed.",
                bundle_path.display()
            )));
        }
        
        // Run XCTest bundle using simctl xctest
        eprintln!("[XCTestCompiler] Running XCTest bundle using simctl xctest...");
        
        // Start the test in the background
        let mut child = Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                "-s",  // standalone mode
                device_id,
                "xctest",
                "-XCTest",
                "ArkavoTestRunner/testRunCommands",
                bundle_path.to_str().unwrap(),
            ])
            .env("ARKAVO_SOCKET_PATH", self.socket_path.to_str().unwrap())
            .spawn()
            .map_err(|e| TestError::Mcp(format!("Failed to spawn xctest: {}", e)))?;
            
        // Give it a moment to start
        std::thread::sleep(std::time::Duration::from_secs(2));
        
        // Check if it's still running
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    eprintln!("[XCTestCompiler] Test completed successfully");
                } else {
                    // Try to get output
                    let output = child.wait_with_output()
                        .map_err(|e| TestError::Mcp(format!("Failed to get test output: {}", e)))?;
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    
                    return Err(TestError::Mcp(format!(
                        "Test runner exited with status: {}. Stdout: {}. Stderr: {}",
                        status, stdout, stderr
                    )));
                }
            }
            Ok(None) => {
                eprintln!("[XCTestCompiler] Test runner is running in background");
                // Test is running, which is what we want for the socket server
            }
            Err(e) => {
                eprintln!("[XCTestCompiler] Warning: Could not check test status: {}", e);
            }
        }
        
        eprintln!("[XCTestCompiler] Test runner started successfully");
        Ok(())
    }
}

/// Find compiled binary in DerivedData
fn find_compiled_binary(derived_data: &Path, name: &str) -> Result<PathBuf> {
    use walkdir::WalkDir;

    for entry in WalkDir::new(derived_data).into_iter().flatten() {
        let path = entry.path();
        if path.file_name() == Some(std::ffi::OsStr::new(name))
            && path.is_file()
            && !path.to_string_lossy().contains(".dSYM")
        {
            return Ok(path.to_path_buf());
        }
    }

    Err(TestError::Mcp(format!(
        "Binary {} not found in DerivedData",
        name
    )))
}
