use crate::{Result, TestError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static XCTEST_BUNDLE_CACHE: OnceLock<PathBuf> = OnceLock::new();

pub struct XCTestCompiler {
    template_dir: PathBuf,
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
        
        // Try multiple methods to find the template directory
        let template_dir = Self::find_template_dir()?;
        
        eprintln!("[XCTestCompiler] Using template directory: {}", template_dir.display());

        let build_dir = std::env::temp_dir().join("arkavo-xctest-build");
        eprintln!("[XCTestCompiler] Build directory: {}", build_dir.display());

        // Create build directory if it doesn't exist
        fs::create_dir_all(&build_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create build directory: {}", e)))?;

        // Generate socket path
        let socket_path =
            std::env::temp_dir().join(format!("arkavo-xctest-{}.sock", std::process::id()));
        eprintln!("[XCTestCompiler] Socket path: {}", socket_path.display());

        Ok(Self {
            template_dir,
            build_dir,
            socket_path,
        })
    }
    
    fn find_template_dir() -> Result<PathBuf> {
        // Method 1: Check if we're in development (templates relative to source)
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dev_template_dir = manifest_dir.join("templates").join("XCTestRunner");
        if dev_template_dir.exists() {
            return Ok(dev_template_dir);
        }
        
        // Method 2: Check relative to executable
        if let Ok(exe_path) = std::env::current_exe() {
            // Try ../share/arkavo/templates (installed location)
            if let Some(exe_dir) = exe_path.parent() {
                let installed_template_dir = exe_dir
                    .parent()
                    .map(|p| p.join("share/arkavo/templates/XCTestRunner"));
                    
                if let Some(dir) = installed_template_dir {
                    if dir.exists() {
                        return Ok(dir);
                    }
                }
                
                // Try ../templates (relative to binary)
                let relative_template_dir = exe_dir.parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.parent())
                    .map(|p| p.join("crates/arkavo-test/templates/XCTestRunner"));
                    
                if let Some(dir) = relative_template_dir {
                    if dir.exists() {
                        return Ok(dir);
                    }
                }
            }
        }
        
        // Method 3: Try current directory structure
        let cwd = std::env::current_dir()
            .map_err(|e| TestError::Mcp(format!("Failed to get current directory: {}", e)))?;
        let cwd_template_dir = cwd.join("crates/arkavo-test/templates/XCTestRunner");
        if cwd_template_dir.exists() {
            return Ok(cwd_template_dir);
        }
        
        Err(TestError::Mcp(format!(
            "Could not find XCTestRunner templates. Searched in:\n\
             1. {} (development)\n\
             2. Relative to executable\n\
             3. {} (current dir)",
            dev_template_dir.display(),
            cwd_template_dir.display()
        )))
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
        eprintln!("[XCTestCompiler] Template dir exists: {}", self.template_dir.exists());

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
        // Process Swift template - try both template files
        let swift_template_enhanced = self.template_dir.join("ArkavoTestRunnerEnhanced.swift.template");
        let swift_template_basic = self.template_dir.join("ArkavoTestRunner.swift.template");
        
        let swift_template_path = if swift_template_enhanced.exists() {
            eprintln!("[XCTestCompiler] Using enhanced Swift template");
            swift_template_enhanced
        } else if swift_template_basic.exists() {
            eprintln!("[XCTestCompiler] Using basic Swift template");
            swift_template_basic
        } else {
            return Err(TestError::Mcp(format!(
                "No Swift template found. Looked for:\n{:?}\n{:?}",
                swift_template_enhanced, swift_template_basic
            )));
        };
        
        let swift_template = fs::read_to_string(&swift_template_path)
            .map_err(|e| TestError::Mcp(format!("Failed to read Swift template at {:?}: {}", swift_template_path, e)))?;

        // Verify this is the updated template
        if swift_template.contains("let result: [String: Any]?") {
            return Err(TestError::Mcp(
                "ERROR: Using outdated Swift template!\n\
                The template contains 'let result: [String: Any]?' which causes Codable errors.\n\
                This should be 'let result: JSONValue?' instead.\n\
                Please rebuild arkavo with the latest source code:\n\
                  cargo build --release --bin arkavo".to_string()
            ));
        }
        
        if !swift_template.contains("enum JSONValue: Codable") {
            return Err(TestError::Mcp(
                "ERROR: Swift template missing JSONValue enum!\n\
                The template needs the JSONValue enum to handle arbitrary JSON.\n\
                Please rebuild arkavo with the latest source code:\n\
                  cargo build --release --bin arkavo".to_string()
            ));
        }

        // Replace template variables
        let swift_source =
            swift_template.replace("{{SOCKET_PATH}}", &self.socket_path.to_string_lossy());

        // Write Swift source
        let swift_path = source_dir.join("ArkavoTestRunner.swift");
        fs::write(&swift_path, swift_source)
            .map_err(|e| TestError::Mcp(format!("Failed to write Swift source: {}", e)))?;

        // Copy Info.plist template
        let plist_template_path = self.template_dir.join("Info.plist.template");
        let plist_content = fs::read_to_string(&plist_template_path)
            .map_err(|e| TestError::Mcp(format!("Failed to read Info.plist template: {}", e)))?;

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
        // First try DerivedData location
        let derived_data = build_dir.join("DerivedData");
        if derived_data.exists() {
            // Search for the compiled binary in DerivedData
            if let Ok(binary_path) = find_compiled_binary(&derived_data, "ArkavoTestRunner") {
                let binary_dst = bundle_path.join("ArkavoTestRunner");
                fs::copy(&binary_path, &binary_dst)
                    .map_err(|e| TestError::Mcp(format!("Failed to copy binary: {}", e)))?;

                // Make it executable
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&binary_dst)?.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&binary_dst, perms)?;
                }
            }
        } else {
            // Try direct compilation output
            let binary_src = build_dir.join("ArkavoTestRunner");
            if binary_src.exists() {
                let binary_dst = bundle_path.join("ArkavoTestRunner");
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
            }
        }

        Ok(bundle_path)
    }

    /// Install the XCTest bundle to a simulator
    pub fn install_to_simulator(&self, device_id: &str, bundle_path: &Path) -> Result<()> {
        // Use xcrun simctl to install the test bundle
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                bundle_path.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to install test bundle: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TestError::Mcp(format!(
                "Failed to install test bundle: {}",
                stderr
            )));
        }

        Ok(())
    }

    /// Run the XCTest bundle on a simulator
    pub fn run_tests(&self, device_id: &str, _bundle_id: &str) -> Result<()> {
        eprintln!("[XCTestCompiler] Running tests on device: {}", device_id);
        
        // First, we need to find where the test bundle is installed
        // XCTest bundles are typically installed in the simulator's test bundles directory
        let test_bundle_name = "ArkavoTestRunner.xctest";
        
        // Method 1: Try to run using xcrun simctl spawn with xctest
        // This will spawn the xctest process directly in the simulator
        eprintln!("[XCTestCompiler] Attempting to run XCTest bundle...");
        
        // We need to spawn a long-running process that will handle our commands
        // The test runner should already be installed and will connect via Unix socket
        let spawn_args = vec![
            "simctl",
            "spawn",
            device_id,
            "xctest",
            "-XCTest",
            "All",
            test_bundle_name,
        ];
        
        eprintln!("[XCTestCompiler] Running: xcrun {}", spawn_args.join(" "));
        
        // Start the test runner in the background
        let mut child = Command::new("xcrun")
            .args(&spawn_args)
            .spawn()
            .map_err(|e| TestError::Mcp(format!("Failed to spawn xctest: {}", e)))?;
            
        // Give it a moment to start
        std::thread::sleep(std::time::Duration::from_secs(2));
        
        // Check if it's still running
        match child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("[XCTestCompiler] XCTest exited with status: {}", status);
                if !status.success() {
                    // Try alternative method: use xcodebuild
                    eprintln!("[XCTestCompiler] Trying xcodebuild method...");
                    return self.run_tests_with_xcodebuild(device_id);
                }
            }
            Ok(None) => {
                eprintln!("[XCTestCompiler] XCTest is running in background");
                // Store the child process handle somewhere if needed
                // For now, we'll just let it run
            }
            Err(e) => {
                eprintln!("[XCTestCompiler] Error checking XCTest status: {}", e);
            }
        }
        
        Ok(())
    }
    
    fn run_tests_with_xcodebuild(&self, device_id: &str) -> Result<()> {
        // Create a minimal xctestrun file
        let xctestrun_content = format!(r#"{{
            "__xctestrun_metadata__": {{
                "FormatVersion": 1
            }},
            "ArkavoTestRunner": {{
                "TestBundlePath": "__TESTROOT__/ArkavoTestRunner.xctest",
                "TestHostPath": "__PLATFORMS__/iPhoneSimulator.platform/Developer/Applications/Simulator.app/Contents/MacOS/Simulator",
                "UITargetAppPath": "__TESTHOST__/ArkavoTestRunner.app",
                "EnvironmentVariables": {{
                    "ARKAVO_SOCKET_PATH": "{}"
                }},
                "TestingEnvironmentVariables": {{
                    "DYLD_FRAMEWORK_PATH": "__TESTROOT__:__PLATFORMS__/iPhoneSimulator.platform/Developer/Library/Frameworks"
                }},
                "OnlyTestIdentifiers": [
                    "ArkavoTestRunner/testRunCommands"
                ]
            }}
        }}"#, self.socket_path.to_string_lossy());
        
        let xctestrun_path = self.build_dir.join("ArkavoTestRunner.xctestrun");
        fs::write(&xctestrun_path, xctestrun_content)
            .map_err(|e| TestError::Mcp(format!("Failed to write xctestrun file: {}", e)))?;
            
        let output = Command::new("xcodebuild")
            .args([
                "test-without-building",
                "-xctestrun", xctestrun_path.to_str().unwrap(),
                "-destination", &format!("id={}", device_id),
                "-resultBundlePath", self.build_dir.join("Results.xcresult").to_str().unwrap(),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run xcodebuild: {}", e)))?;
            
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[XCTestCompiler] xcodebuild failed: {}", stderr);
            return Err(TestError::Mcp(format!("Failed to run tests with xcodebuild: {}", stderr)));
        }
        
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
