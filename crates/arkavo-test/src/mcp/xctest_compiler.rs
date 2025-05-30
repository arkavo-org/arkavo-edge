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
        let template_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("templates")
            .join("XCTestRunner");

        let build_dir = std::env::temp_dir().join("arkavo-xctest-build");

        // Create build directory if it doesn't exist
        fs::create_dir_all(&build_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create build directory: {}", e)))?;

        // Generate socket path
        let socket_path =
            std::env::temp_dir().join(format!("arkavo-xctest-{}.sock", std::process::id()));

        Ok(Self {
            template_dir,
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
        eprintln!("Compiling XCTest bundle...");

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
        // Process Swift template
        let swift_template_path = self.template_dir.join("ArkavoTestRunner.swift.template");
        let swift_template = fs::read_to_string(&swift_template_path)
            .map_err(|e| TestError::Mcp(format!("Failed to read Swift template: {}", e)))?;

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
        // Use xcodebuild to compile for iOS Simulator
        let output = Command::new("xcodebuild")
            .args([
                "build",
                "-scheme",
                "ArkavoTestRunner",
                "-destination",
                "generic/platform=iOS Simulator",
                "-derivedDataPath",
                build_dir.join("DerivedData").to_str().unwrap(),
                "-configuration",
                "Debug",
                "CODE_SIGNING_ALLOWED=NO",
                "CODE_SIGN_IDENTITY=-",
            ])
            .current_dir(build_dir)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run xcodebuild: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Fallback: Try using swift directly to compile
            eprintln!("xcodebuild failed, trying direct swift compilation...");

            let swift_output = Command::new("xcrun")
                .args([
                    "swiftc",
                    "-sdk",
                    "iphonesimulator",
                    "-target",
                    "x86_64-apple-ios15.0-simulator",
                    "-emit-library",
                    "-emit-module",
                    "-module-name",
                    "ArkavoTestRunner",
                    "-Xlinker",
                    "-bundle",
                    "-o",
                    build_dir.join("ArkavoTestRunner").to_str().unwrap(),
                    build_dir
                        .join("Sources/ArkavoTestRunner.swift")
                        .to_str()
                        .unwrap(),
                ])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to run swift compiler: {}", e)))?;

            if !swift_output.status.success() {
                let swift_stderr = String::from_utf8_lossy(&swift_output.stderr);
                return Err(TestError::Mcp(format!(
                    "Compilation failed. xcodebuild error: {}\nswift error: {}",
                    stderr, swift_stderr
                )));
            }
        }

        Ok(())
    }

    /// Create the .xctest bundle structure
    fn create_xctest_bundle(&self, build_dir: &Path) -> Result<PathBuf> {
        let bundle_name = "ArkavoTestRunner.xctest";
        let bundle_path = build_dir.join(bundle_name);

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
    pub fn run_tests(&self, device_id: &str, bundle_id: &str) -> Result<()> {
        // Use xcodebuild to run the tests
        let output = Command::new("xcodebuild")
            .args([
                "test-without-building",
                "-xctestrun", // This would need a proper test run file
                "-destination",
                &format!("id={}", device_id),
                "-only-testing",
                &format!("{}/ArkavoTestRunner/testRunCommands", bundle_id),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run tests: {}", e)))?;

        if !output.status.success() {
            // Fallback: Use xcrun simctl xctest
            let xctest_output = Command::new("xcrun")
                .args([
                    "simctl",
                    "spawn",
                    device_id,
                    "xctest",
                    "-XCTest",
                    "ArkavoTestRunner/testRunCommands",
                    "/path/to/ArkavoTestRunner.xctest", // This needs the installed path
                ])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to run xctest: {}", e)))?;

            if !xctest_output.status.success() {
                let stderr = String::from_utf8_lossy(&xctest_output.stderr);
                return Err(TestError::Mcp(format!("Failed to run tests: {}", stderr)));
            }
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
