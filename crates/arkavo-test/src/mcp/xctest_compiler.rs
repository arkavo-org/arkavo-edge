use super::templates;
use crate::{Result, TestError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// Disabled caching due to architecture issues
// static XCTEST_BUNDLE_CACHE: OnceLock<PathBuf> = OnceLock::new();

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
            .map_err(|e| {
                TestError::Mcp(format!(
                    "xcrun not found. Xcode or Xcode Command Line Tools must be installed.\n\
                Install Xcode from the App Store or run: xcode-select --install\n\
                Error: {}",
                    e
                ))
            })?;

        if !xcode_check.status.success() {
            return Err(TestError::Mcp(
                "xcrun failed. Make sure Xcode Command Line Tools are properly configured.\n\
                Run: sudo xcode-select --switch /Applications/Xcode.app\n\
                Or: xcode-select --install"
                    .to_string(),
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
        // Always compile fresh to avoid architecture mismatches
        eprintln!("[XCTestCompiler] Compiling fresh XCTest bundle (caching disabled)");
        let bundle_path = self.compile_xctest_bundle()?;
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
        debug_assert!(
            !swift_template.contains("let result: [String: Any]?"),
            "Embedded template should not contain [String: Any]"
        );
        debug_assert!(
            swift_template.contains("enum JSONValue: Codable"),
            "Embedded template should contain JSONValue enum"
        );

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

    /// Compile the Swift package as an executable instead of a bundle
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

        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout)
            .trim()
            .to_string();
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

        let platform_path = String::from_utf8_lossy(&platform_output.stdout)
            .trim()
            .to_string();
        let xctest_framework_path = format!("{}/Developer/Library/Frameworks", platform_path);
        eprintln!(
            "[XCTestCompiler] XCTest framework path: {}",
            xctest_framework_path
        );

        // Verify XCTest framework exists
        if !std::path::Path::new(&format!("{}/XCTest.framework", xctest_framework_path)).exists() {
            return Err(TestError::Mcp(format!(
                "XCTest.framework not found at {}. Xcode may not be properly installed.",
                xctest_framework_path
            )));
        }

        // Only support ARM64 simulators
        let target = "arm64-apple-ios15.0-simulator";
        eprintln!("[XCTestCompiler] Compiling for architecture: {}", target);

        // Compile as a framework/bundle
        let output = Command::new("xcrun")
            .args([
                "swiftc",
                "-sdk",
                &sdk_path,
                "-target",
                target,
                "-emit-library",
                "-emit-module",
                "-module-name",
                "ArkavoTestRunner",
                "-Xlinker",
                "-bundle",
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                "@executable_path/Frameworks",
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                "@loader_path/Frameworks",
                "-F",
                &xctest_framework_path,
                "-F",
                &sdk_path,
                "-framework",
                "XCTest",
                "-o",
                output_binary.to_str().unwrap(),
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

            return Err(TestError::Mcp(format!(
                "Swift compilation failed for ARM64.\nError: {}\n\nNote: Only ARM64 simulators are supported.",
                stderr
            )));
        }

        Ok(())
    }

    /// Create the .xctest bundle structure
    fn create_xctest_bundle(&self, build_dir: &Path) -> Result<PathBuf> {
        let bundle_name = "ArkavoTestRunner.xctest";
        let bundle_path = build_dir.join(bundle_name);

        eprintln!(
            "[XCTestCompiler] Creating bundle at: {}",
            bundle_path.display()
        );

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
            eprintln!(
                "[XCTestCompiler] Copying binary from {} to {}",
                binary_src.display(),
                binary_dst.display()
            );
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
                    eprintln!(
                        "[XCTestCompiler] Found binary in DerivedData at {}",
                        binary_path.display()
                    );
                    fs::copy(&binary_path, &binary_dst).map_err(|e| {
                        TestError::Mcp(format!("Failed to copy binary from DerivedData: {}", e))
                    })?;

                    // Make it executable
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut perms = fs::metadata(&binary_dst)?.permissions();
                        perms.set_mode(0o755);
                        fs::set_permissions(&binary_dst, perms)?;
                    }
                } else {
                    return Err(TestError::Mcp(
                        "Compiled binary not found in build directory or DerivedData".to_string(),
                    ));
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
        eprintln!(
            "[XCTestCompiler] Installing XCTest bundle to simulator {}...",
            device_id
        );

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
        let test_bundles_dir =
            sim_data_path.join("Library/Developer/Xcode/DerivedData/TestBundles");

        // Create the directory if it doesn't exist
        fs::create_dir_all(&test_bundles_dir).map_err(|e| {
            TestError::Mcp(format!("Failed to create test bundles directory: {}", e))
        })?;

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
            .args([
                "-R",
                bundle_path.to_str().unwrap(),
                dest_path.parent().unwrap().to_str().unwrap(),
            ])
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

    /// Create a minimal host app that can load and run our XCTest bundle
    fn create_test_host_app(&self) -> Result<PathBuf> {
        let app_dir = self.build_dir.join("ArkavoTestHost.app");

        // Remove existing app if it exists to ensure clean build
        if app_dir.exists() {
            eprintln!("[XCTestCompiler] Removing existing test host app to ensure clean build");
            fs::remove_dir_all(&app_dir)?;
        }

        fs::create_dir_all(&app_dir)?;

        // Create Info.plist with deep link support for calibration
        let info_plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>ArkavoTestHost</string>
    <key>CFBundleIdentifier</key>
    <string>com.arkavo.testhost</string>
    <key>CFBundleName</key>
    <string>ArkavoTestHost</string>
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
    <key>UIApplicationSupportsIndirectInputEvents</key>
    <true/>
    <key>UIStatusBarHidden</key>
    <true/>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLSchemes</key>
            <array>
                <string>arkavo-edge</string>
                <string>arkavo-test</string>
            </array>
            <key>CFBundleURLName</key>
            <string>com.arkavo.testhost</string>
        </dict>
    </array>
</dict>
</plist>"#;

        let plist_path = app_dir.join("Info.plist");
        fs::write(&plist_path, info_plist)?;
        eprintln!(
            "[XCTestCompiler] Wrote Info.plist to: {}",
            plist_path.display()
        );

        // Verify the URL types were written correctly
        let written_content = fs::read_to_string(&plist_path)?;
        if written_content.contains("CFBundleURLTypes") {
            eprintln!("[XCTestCompiler] ✓ Info.plist contains URL types for deep links");
        } else {
            eprintln!("[XCTestCompiler] ⚠️ Warning: Info.plist missing URL types!");
        }

        // Create a minimal executable that loads and runs our test
        let main_m = r#"
#import <UIKit/UIKit.h>
#import <XCTest/XCTest.h>
#import <dlfcn.h>

@interface TestBridge : NSObject
+ (void)setupBridge;
@end

@implementation TestBridge

+ (void)setupBridge {
    // Check for both direct and SIMCTL_CHILD_ prefixed environment variables
    NSString *socketPath = [[[NSProcessInfo processInfo] environment] objectForKey:@"ARKAVO_SOCKET_PATH"];
    if (!socketPath) {
        socketPath = [[[NSProcessInfo processInfo] environment] objectForKey:@"SIMCTL_CHILD_ARKAVO_SOCKET_PATH"];
    }
    
    NSString *bundlePath = [[[NSProcessInfo processInfo] environment] objectForKey:@"ARKAVO_TEST_BUNDLE"];
    if (!bundlePath) {
        bundlePath = [[[NSProcessInfo processInfo] environment] objectForKey:@"SIMCTL_CHILD_ARKAVO_TEST_BUNDLE"];
    }
    
    NSString *targetAppId = [[[NSProcessInfo processInfo] environment] objectForKey:@"ARKAVO_TARGET_APP_ID"];
    if (!targetAppId) {
        targetAppId = [[[NSProcessInfo processInfo] environment] objectForKey:@"SIMCTL_CHILD_ARKAVO_TARGET_APP_ID"];
    }
    
    NSLog(@"[TestBridge] Setting up bridge...");
    NSLog(@"[TestBridge] Socket path: %@", socketPath ?: @"not set");
    NSLog(@"[TestBridge] Bundle path: %@", bundlePath ?: @"not set");
    NSLog(@"[TestBridge] Target app: %@", targetAppId ?: @"not set");
    
    if (!bundlePath) {
        NSLog(@"[TestBridge] ERROR: ARKAVO_TEST_BUNDLE not set");
        return;
    }
    
    // Resolve relative path if needed
    NSString *resolvedPath = bundlePath;
    if (![bundlePath hasPrefix:@"/"]) {
        NSBundle *mainBundle = [NSBundle mainBundle];
        resolvedPath = [[mainBundle bundlePath] stringByAppendingPathComponent:bundlePath];
        NSLog(@"[TestBridge] Resolved relative path to: %@", resolvedPath);
    }
    
    // Check if bundle exists
    NSFileManager *fm = [NSFileManager defaultManager];
    if (![fm fileExistsAtPath:resolvedPath]) {
        NSLog(@"[TestBridge] ERROR: Test bundle not found at: %@", resolvedPath);
        return;
    }
    
    // Load the test bundle
    NSBundle *testBundle = [NSBundle bundleWithPath:resolvedPath];
    if (!testBundle) {
        NSLog(@"[TestBridge] ERROR: Failed to create bundle from path");
        return;
    }
    
    NSError *error = nil;
    if (![testBundle loadAndReturnError:&error]) {
        NSLog(@"[TestBridge] ERROR: Failed to load bundle: %@", error);
        NSLog(@"[TestBridge] Bundle path exists: %d", [fm fileExistsAtPath:resolvedPath]);
        NSLog(@"[TestBridge] Bundle executable: %@", [testBundle executablePath]);
        NSLog(@"[TestBridge] Bundle loaded: %d", [testBundle isLoaded]);
        return;
    }
    
    NSLog(@"[TestBridge] Test bundle loaded successfully");
    NSLog(@"[TestBridge] Bundle executable: %@", [testBundle executablePath]);
    NSLog(@"[TestBridge] Bundle principal class: %@", NSStringFromClass([testBundle principalClass]));
    
    // Get the principal class (should be ArkavoTestRunner)
    Class testClass = [testBundle principalClass];
    if (!testClass) {
        testClass = [testBundle classNamed:@"ArkavoTestRunner"];
    }
    
    if (testClass) {
        NSLog(@"[TestBridge] Found test class: %@", NSStringFromClass(testClass));
        
        // Call setUp to initialize the socket server
        if ([testClass respondsToSelector:@selector(setUp)]) {
            NSLog(@"[TestBridge] Calling test class setUp...");
            [testClass performSelector:@selector(setUp)];
        }
        
        // Call initializeBridge to launch target app if needed
        if ([testClass respondsToSelector:@selector(initializeBridge)]) {
            NSLog(@"[TestBridge] Calling test class initializeBridge...");
            [testClass performSelector:@selector(initializeBridge)];
        }
        
        NSLog(@"[TestBridge] Bridge setup complete - test infrastructure ready");
    } else {
        NSLog(@"[TestBridge] ERROR: Could not find test class in bundle");
    }
}

@end

// Calibration view controller with coordinate display
@interface CalibrationViewController : UIViewController
@property (nonatomic, strong) UILabel *titleLabel;
@property (nonatomic, strong) UILabel *instructionLabel;
@property (nonatomic, strong) UILabel *coordinateLabel;
@property (nonatomic, strong) UILabel *lastTapLabel;
@property (nonatomic, strong) NSMutableArray *calibrationTargets;
@property (nonatomic, strong) NSMutableArray *tapResults;
@property (nonatomic, strong) NSMutableArray *tapMarkers;
@property (nonatomic, assign) NSInteger currentTargetIndex;
@property (nonatomic, strong) UIView *targetView;
@property (nonatomic, strong) NSString *resultsFilePath;
@property (nonatomic, assign) BOOL waitingForTap;
@end

@implementation CalibrationViewController

- (instancetype)init {
    self = [super init];
    if (self) {
        self.calibrationTargets = [NSMutableArray array];
        self.tapResults = [NSMutableArray array];
        self.tapMarkers = [NSMutableArray array];
        self.currentTargetIndex = 0;
        self.waitingForTap = YES;
        
        // Setup results file path
        NSArray *paths = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES);
        NSString *documentsPath = [paths firstObject];
        self.resultsFilePath = [documentsPath stringByAppendingPathComponent:@"calibration_results.json"];
    }
    return self;
}

- (void)viewDidLoad {
    [super viewDidLoad];
    
    NSLog(@"[Calibration] View did load");
    self.view.backgroundColor = [UIColor whiteColor];
    
    // Title
    self.titleLabel = [[UILabel alloc] init];
    self.titleLabel.text = @"Calibration Mode";
    self.titleLabel.font = [UIFont boldSystemFontOfSize:24];
    self.titleLabel.textAlignment = NSTextAlignmentCenter;
    self.titleLabel.translatesAutoresizingMaskIntoConstraints = NO;
    [self.view addSubview:self.titleLabel];
    
    // Instructions
    self.instructionLabel = [[UILabel alloc] init];
    self.instructionLabel.text = @"Waiting for automated taps...";
    self.instructionLabel.textAlignment = NSTextAlignmentCenter;
    self.instructionLabel.translatesAutoresizingMaskIntoConstraints = NO;
    [self.view addSubview:self.instructionLabel];
    
    // Coordinate display - LARGE and prominent
    self.coordinateLabel = [[UILabel alloc] init];
    self.coordinateLabel.text = @"TAP COORDINATES";
    self.coordinateLabel.font = [UIFont boldSystemFontOfSize:32];
    self.coordinateLabel.textAlignment = NSTextAlignmentCenter;
    self.coordinateLabel.backgroundColor = [UIColor blackColor];
    self.coordinateLabel.textColor = [UIColor whiteColor];
    self.coordinateLabel.layer.cornerRadius = 10;
    self.coordinateLabel.layer.masksToBounds = YES;
    self.coordinateLabel.translatesAutoresizingMaskIntoConstraints = NO;
    [self.view addSubview:self.coordinateLabel];
    
    // Last tap info
    self.lastTapLabel = [[UILabel alloc] init];
    self.lastTapLabel.text = @"No taps yet";
    self.lastTapLabel.font = [UIFont systemFontOfSize:18];
    self.lastTapLabel.textAlignment = NSTextAlignmentCenter;
    self.lastTapLabel.translatesAutoresizingMaskIntoConstraints = NO;
    [self.view addSubview:self.lastTapLabel];
    
    // Layout
    [NSLayoutConstraint activateConstraints:@[
        [self.titleLabel.topAnchor constraintEqualToAnchor:self.view.safeAreaLayoutGuide.topAnchor constant:20],
        [self.titleLabel.centerXAnchor constraintEqualToAnchor:self.view.centerXAnchor],
        
        [self.instructionLabel.topAnchor constraintEqualToAnchor:self.titleLabel.bottomAnchor constant:10],
        [self.instructionLabel.centerXAnchor constraintEqualToAnchor:self.view.centerXAnchor],
        
        [self.coordinateLabel.centerXAnchor constraintEqualToAnchor:self.view.centerXAnchor],
        [self.coordinateLabel.centerYAnchor constraintEqualToAnchor:self.view.centerYAnchor],
        [self.coordinateLabel.widthAnchor constraintEqualToConstant:350],
        [self.coordinateLabel.heightAnchor constraintEqualToConstant:80],
        
        [self.lastTapLabel.topAnchor constraintEqualToAnchor:self.coordinateLabel.bottomAnchor constant:20],
        [self.lastTapLabel.centerXAnchor constraintEqualToAnchor:self.view.centerXAnchor]
    ]];
    
    // Start calibration immediately
    NSLog(@"[Calibration] Ready to receive taps");
    [self showNextTarget];
}

- (void)showNextTarget {
    self.waitingForTap = YES;
    self.instructionLabel.text = @"Waiting for next tap...";
    self.coordinateLabel.text = @"READY FOR TAP";
    self.coordinateLabel.backgroundColor = [UIColor blackColor];
    NSLog(@"[Calibration] showNextTarget - waiting for tap %ld", (long)self.tapResults.count + 1);
}

- (void)touchesBegan:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event {
    NSLog(@"[Calibration] touchesBegan called - waitingForTap: %@", self.waitingForTap ? @"YES" : @"NO");
    
    if (!self.waitingForTap) return;
    
    UITouch *touch = [touches anyObject];
    CGPoint tapLocation = [touch locationInView:self.view];
    NSLog(@"[Calibration] Tap detected at (%.0f, %.0f)", tapLocation.x, tapLocation.y);
    
    // Show tap coordinates prominently
    self.coordinateLabel.text = [NSString stringWithFormat:@"X: %.0f  Y: %.0f", tapLocation.x, tapLocation.y];
    self.coordinateLabel.backgroundColor = [UIColor systemGreenColor];
    self.coordinateLabel.textColor = [UIColor blackColor];
    
    // Update last tap info
    self.lastTapLabel.text = [NSString stringWithFormat:@"Tap %ld at (%.0f, %.0f)", 
                              (long)self.tapResults.count + 1, tapLocation.x, tapLocation.y];
    
    // Create visual marker at tap location
    UIView *tapMarker = [[UIView alloc] initWithFrame:CGRectMake(tapLocation.x - 15, tapLocation.y - 15, 30, 30)];
    tapMarker.backgroundColor = [UIColor systemGreenColor];
    tapMarker.layer.cornerRadius = 15;
    tapMarker.layer.borderWidth = 2;
    tapMarker.layer.borderColor = [UIColor whiteColor].CGColor;
    tapMarker.alpha = 0.8;
    [self.view addSubview:tapMarker];
    [self.tapMarkers addObject:tapMarker];
    
    // Add coordinate label at tap location
    UILabel *coordLabel = [[UILabel alloc] init];
    coordLabel.text = [NSString stringWithFormat:@"(%.0f,%.0f)", tapLocation.x, tapLocation.y];
    coordLabel.font = [UIFont boldSystemFontOfSize:14];
    coordLabel.textColor = [UIColor blackColor];
    coordLabel.backgroundColor = [UIColor whiteColor];
    coordLabel.layer.cornerRadius = 4;
    coordLabel.layer.masksToBounds = YES;
    coordLabel.translatesAutoresizingMaskIntoConstraints = NO;
    [self.view addSubview:coordLabel];
    [self.tapMarkers addObject:coordLabel];
    
    [NSLayoutConstraint activateConstraints:@[
        [coordLabel.centerXAnchor constraintEqualToAnchor:tapMarker.centerXAnchor],
        [coordLabel.topAnchor constraintEqualToAnchor:tapMarker.bottomAnchor constant:5]
    ]];
    
    // Record the result
    NSDictionary *result = @{
        @"timestamp": [NSDate date],
        @"actual": @{@"x": @(tapLocation.x), @"y": @(tapLocation.y)},
        @"targetHit": @YES
    };
    [self.tapResults addObject:result];
    
    // Save after each tap for real-time access
    [self saveCalibrationResults];
    
    self.waitingForTap = NO;
    
    // Check if we've collected enough taps
    if (self.tapResults.count >= 5) {
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(2.0 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [self calibrationComplete];
        });
    } else {
        // Ready for next tap after a short delay
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.0 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [self showNextTarget];
        });
    }
}

- (void)calibrationComplete {
    self.instructionLabel.text = @"Calibration Complete!";
    self.coordinateLabel.text = @"FINISHED";
    self.coordinateLabel.backgroundColor = [UIColor systemBlueColor];
    self.coordinateLabel.textColor = [UIColor whiteColor];
    
    // Save final results
    [self saveCalibrationResults];
    
    // Keep the view open to show results
    self.lastTapLabel.text = [NSString stringWithFormat:@"Collected %ld tap coordinates", (long)self.tapResults.count];
}

- (void)saveCalibrationResults {
    NSDictionary *results = @{
        @"device_info": [[UIDevice currentDevice] model],
        @"screen_size": @{
            @"width": @(self.view.bounds.size.width),
            @"height": @(self.view.bounds.size.height)
        },
        @"tap_events": self.tapResults,
        @"calibration_complete": @YES
    };
    
    NSError *error;
    NSData *jsonData = [NSJSONSerialization dataWithJSONObject:results options:NSJSONWritingPrettyPrinted error:&error];
    
    if (!error) {
        [jsonData writeToFile:self.resultsFilePath atomically:YES];
        NSLog(@"[Calibration] Results saved to: %@", self.resultsFilePath);
    }
}

@end

// Enhanced app delegate with calibration support
@interface TestHostAppDelegate : UIResponder <UIApplicationDelegate>
@property (strong, nonatomic) UIWindow *window;
@property (strong, nonatomic) UINavigationController *navigationController;
@property (nonatomic, assign) BOOL isInCalibrationMode;
@end

@implementation TestHostAppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    NSLog(@"[TestHost] App launched - setting up test bridge immediately...");
    
    // Set up the bridge immediately to ensure socket is ready when Rust connects
    [TestBridge setupBridge];
    
    // Create window and root view controller
    self.window = [[UIWindow alloc] initWithFrame:[[UIScreen mainScreen] bounds]];
    
    // Create a minimal root view controller
    UIViewController *rootVC = [[UIViewController alloc] init];
    rootVC.view.backgroundColor = [UIColor blackColor];
    
    UILabel *label = [[UILabel alloc] init];
    label.text = @"ArkavoTestHost";
    label.textColor = [UIColor whiteColor];
    label.textAlignment = NSTextAlignmentCenter;
    label.translatesAutoresizingMaskIntoConstraints = NO;
    [rootVC.view addSubview:label];
    
    [NSLayoutConstraint activateConstraints:@[
        [label.centerXAnchor constraintEqualToAnchor:rootVC.view.centerXAnchor],
        [label.centerYAnchor constraintEqualToAnchor:rootVC.view.centerYAnchor]
    ]];
    
    self.navigationController = [[UINavigationController alloc] initWithRootViewController:rootVC];
    self.navigationController.navigationBarHidden = YES;
    
    self.window.rootViewController = self.navigationController;
    [self.window makeKeyAndVisible];
    
    // Log target app if specified
    NSString *targetAppId = [[[NSProcessInfo processInfo] environment] objectForKey:@"ARKAVO_TARGET_APP_ID"];
    if (!targetAppId) {
        targetAppId = [[[NSProcessInfo processInfo] environment] objectForKey:@"SIMCTL_CHILD_ARKAVO_TARGET_APP_ID"];
    }
    
    if (targetAppId) {
        NSLog(@"[TestHost] Target app specified: %@", targetAppId);
    }
    
    // Check for calibration mode via environment variable instead of deep link
    NSString *calibrationMode = [[[NSProcessInfo processInfo] environment] objectForKey:@"ARKAVO_CALIBRATION_MODE"];
    if (!calibrationMode) {
        calibrationMode = [[[NSProcessInfo processInfo] environment] objectForKey:@"SIMCTL_CHILD_ARKAVO_CALIBRATION_MODE"];
    }
    
    if (calibrationMode && [calibrationMode isEqualToString:@"1"]) {
        NSLog(@"[TestHost] Calibration mode enabled via environment variable");
        [self enterCalibrationMode];
    } else {
        // Check if launched for calibration via deep link (fallback)
        NSURL *url = launchOptions[UIApplicationLaunchOptionsURLKey];
        if (url && [[url scheme] isEqualToString:@"arkavo-edge"] && [[url host] isEqualToString:@"calibration"]) {
            [self enterCalibrationMode];
        } else {
            // Move to background after a delay if not in calibration mode
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.0 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
                if (!self.isInCalibrationMode) {
                    NSLog(@"[TestHost] Moving to background...");
                    [[UIApplication sharedApplication] performSelector:@selector(suspend)];
                }
            });
        }
    }
    
    return YES;
}

- (BOOL)application:(UIApplication *)app openURL:(NSURL *)url options:(NSDictionary<UIApplicationOpenURLOptionsKey,id> *)options {
    NSLog(@"[TestHost] Open URL: %@", url);
    
    if ([[url scheme] isEqualToString:@"arkavo-edge"] && [[url host] isEqualToString:@"calibration"]) {
        [self enterCalibrationMode];
        return YES;
    }
    
    return NO;
}

- (void)enterCalibrationMode {
    NSLog(@"[TestHost] Entering calibration mode");
    self.isInCalibrationMode = YES;
    
    // Present calibration view modally to ensure it's visible
    CalibrationViewController *calibrationVC = [[CalibrationViewController alloc] init];
    UINavigationController *navController = [[UINavigationController alloc] initWithRootViewController:calibrationVC];
    navController.modalPresentationStyle = UIModalPresentationFullScreen;
    
    // Present from the root view controller
    [self.window.rootViewController presentViewController:navController animated:NO completion:^{
        NSLog(@"[TestHost] Calibration view presented");
    }];
}

@end

int main(int argc, char * argv[]) {
    @autoreleasepool {
        return UIApplicationMain(argc, argv, nil, NSStringFromClass([TestHostAppDelegate class]));
    }
}
"#;

        let main_path = self.build_dir.join("main.m");
        fs::write(&main_path, main_m)?;

        // Compile the host app
        let sdk_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
            .output()?;

        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout)
            .trim()
            .to_string();

        // Get platform path for frameworks
        let platform_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-platform-path"])
            .output()?;

        let platform_path = String::from_utf8_lossy(&platform_output.stdout)
            .trim()
            .to_string();
        let xctest_framework_path = format!("{}/Developer/Library/Frameworks", platform_path);

        let compile_output = Command::new("xcrun")
            .args([
                "clang",
                "-fobjc-arc",
                "-framework",
                "UIKit",
                "-framework",
                "XCTest",
                "-isysroot",
                &sdk_path,
                "-F",
                &xctest_framework_path,
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                &xctest_framework_path,
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                "@executable_path/Frameworks",
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                "@loader_path/Frameworks",
                "-target",
                "arm64-apple-ios15.0-simulator",
                "-o",
                app_dir.join("ArkavoTestHost").to_str().unwrap(),
                main_path.to_str().unwrap(),
            ])
            .output()?;

        if !compile_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to compile test host app: {}",
                String::from_utf8_lossy(&compile_output.stderr)
            )));
        }

        // Sign the app with ad-hoc signature (no special entitlements)
        let _ = Command::new("codesign")
            .args(["--force", "--sign", "-", app_dir.to_str().unwrap()])
            .output();

        Ok(app_dir)
    }

    /// Run the XCTest bundle on a simulator
    ///
    /// # Arguments
    /// Launch the test host app (not as a test, but as a regular app)
    pub fn launch_test_host(
        &self,
        device_id: &str,
        target_app_bundle_id: Option<&str>,
    ) -> Result<()> {
        eprintln!(
            "[XCTestCompiler] Launching test host app on device: {}",
            device_id
        );
        eprintln!(
            "[XCTestCompiler] Socket path: {}",
            self.socket_path.display()
        );

        // Get the compiled bundle path
        let bundle_path = self.build_dir.join("ArkavoTestRunner.xctest");
        if !bundle_path.exists() {
            return Err(TestError::Mcp(format!(
                "Test bundle not found at {}. Compilation may have failed.",
                bundle_path.display()
            )));
        }

        // Create and install the host app
        eprintln!("[XCTestCompiler] Creating test host app...");
        let host_app_path = self.create_test_host_app()?;

        // Copy the test bundle into the host app
        let host_app_bundle_path = host_app_path.join("ArkavoTestRunner.xctest");
        eprintln!("[XCTestCompiler] Embedding test bundle into host app...");
        let copy_result = Command::new("cp")
            .args([
                "-R",
                bundle_path.to_str().unwrap(),
                host_app_bundle_path.to_str().unwrap(),
            ])
            .output()?;

        if !copy_result.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to copy test bundle into host app: {}",
                String::from_utf8_lossy(&copy_result.stderr)
            )));
        }

        // Uninstall existing host app if present to ensure clean install
        eprintln!("[XCTestCompiler] Uninstalling existing host app if present...");
        let _ = Command::new("xcrun")
            .args(["simctl", "uninstall", device_id, "com.arkavo.testhost"])
            .output();

        // Install the host app
        eprintln!("[XCTestCompiler] Installing host app...");
        eprintln!(
            "[XCTestCompiler] Host app path: {}",
            host_app_path.display()
        );

        // Verify Info.plist contains URL types before installing
        let info_plist_path = host_app_path.join("Info.plist");
        if let Ok(plist_content) = fs::read_to_string(&info_plist_path) {
            if plist_content.contains("arkavo-edge") {
                eprintln!("[XCTestCompiler] ✓ Info.plist contains arkavo-edge URL scheme");
            } else {
                eprintln!(
                    "[XCTestCompiler] ⚠️ WARNING: Info.plist missing arkavo-edge URL scheme!"
                );
            }
        }

        let install_output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                host_app_path.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to install host app: {}", e)))?;

        if !install_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to install test host app: {}",
                String::from_utf8_lossy(&install_output.stderr)
            )));
        }

        // Launch the host app as a regular app with environment variables
        eprintln!("[XCTestCompiler] Launching test host app...");

        // Set up environment variables for the child process
        let mut cmd = Command::new("xcrun");
        cmd.args(["simctl", "launch", device_id, "com.arkavo.testhost"]);

        // Environment variables must be set with SIMCTL_CHILD_ prefix
        cmd.env(
            "SIMCTL_CHILD_ARKAVO_SOCKET_PATH",
            self.socket_path.display().to_string(),
        );
        cmd.env(
            "SIMCTL_CHILD_ARKAVO_TEST_BUNDLE",
            self.build_dir
                .join("ArkavoTestRunner.xctest")
                .display()
                .to_string(),
        );

        if let Some(app_id) = target_app_bundle_id {
            cmd.env("SIMCTL_CHILD_ARKAVO_TARGET_APP_ID", app_id);
        }

        let launch_output = cmd
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to launch host app: {}", e)))?;

        if !launch_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to launch test host app: {}",
                String::from_utf8_lossy(&launch_output.stderr)
            )));
        }

        eprintln!("[XCTestCompiler] Test host app launched successfully");
        eprintln!(
            "[XCTestCompiler] Host app PID: {}",
            String::from_utf8_lossy(&launch_output.stdout).trim()
        );

        Ok(())
    }

    /// * `device_id` - The simulator device ID
    /// * `target_app_bundle_id` - Optional bundle ID of the app to test
    #[deprecated(note = "Use launch_test_host instead to avoid paradigm mixing")]
    pub fn run_tests(&self, device_id: &str, target_app_bundle_id: Option<&str>) -> Result<()> {
        eprintln!("[XCTestCompiler] Running tests on device: {}", device_id);
        eprintln!(
            "[XCTestCompiler] Socket path: {}",
            self.socket_path.display()
        );

        if let Some(app_id) = target_app_bundle_id {
            eprintln!("[XCTestCompiler] Target app bundle ID: {}", app_id);

            // First, launch the target app
            eprintln!("[XCTestCompiler] Launching target app: {}", app_id);
            let launch_output = Command::new("xcrun")
                .args(["simctl", "launch", device_id, app_id])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to launch target app: {}", e)))?;

            if !launch_output.status.success() {
                eprintln!(
                    "[XCTestCompiler] Warning: Failed to launch target app: {}",
                    String::from_utf8_lossy(&launch_output.stderr)
                );
                // Continue anyway - the app might already be running
            }

            // Give the app time to start
            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        // Get the compiled bundle path
        let bundle_path = self.build_dir.join("ArkavoTestRunner.xctest");
        if !bundle_path.exists() {
            return Err(TestError::Mcp(format!(
                "Test bundle not found at {}. Compilation may have failed.",
                bundle_path.display()
            )));
        }

        // Back to the test host app approach, but we'll make it minimally invasive
        eprintln!("[XCTestCompiler] Creating test host app...");
        let host_app_path = self.create_test_host_app()?;

        // Install the host app
        let install_output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                host_app_path.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to install host app: {}", e)))?;

        if !install_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to install test host app: {}",
                String::from_utf8_lossy(&install_output.stderr)
            )));
        }

        // Instead of hardcoding paths, let's copy the test bundle into the host app's bundle
        // This makes it portable across different environments
        let host_app_bundle_path = host_app_path.join("ArkavoTestRunner.xctest");

        // Copy the test bundle into the host app
        eprintln!("[XCTestCompiler] Copying test bundle into host app...");
        let copy_result = Command::new("cp")
            .args([
                "-R",
                bundle_path.to_str().unwrap(),
                host_app_bundle_path.to_str().unwrap(),
            ])
            .output()?;

        if !copy_result.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to copy test bundle into host app: {}",
                String::from_utf8_lossy(&copy_result.stderr)
            )));
        }

        // Now reinstall the host app with the embedded test bundle
        eprintln!("[XCTestCompiler] Reinstalling host app with embedded test bundle...");
        let reinstall_output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                host_app_path.to_str().unwrap(),
            ])
            .output()?;

        if !reinstall_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to reinstall host app: {}",
                String::from_utf8_lossy(&reinstall_output.stderr)
            )));
        }

        // The test bundle is now at a predictable location relative to the app
        let relative_bundle_path = "ArkavoTestRunner.xctest";

        eprintln!("[XCTestCompiler] Launching test host app...");

        // Launch with environment variables and console output
        eprintln!("[XCTestCompiler] Launching with environment:");
        eprintln!("  ARKAVO_SOCKET_PATH={}", self.socket_path.display());
        eprintln!("  ARKAVO_TEST_BUNDLE={}", relative_bundle_path);
        if let Some(app_id) = target_app_bundle_id {
            eprintln!("  ARKAVO_TARGET_APP_ID={}", app_id);
        }

        let mut launch_cmd = Command::new("xcrun");
        launch_cmd
            .args([
                "simctl",
                "launch",
                "--terminate-running-process",
                device_id,
                "com.arkavo.testhost",
            ])
            .env(
                "SIMCTL_CHILD_ARKAVO_SOCKET_PATH",
                self.socket_path.to_str().unwrap(),
            )
            .env("SIMCTL_CHILD_ARKAVO_TEST_BUNDLE", relative_bundle_path);

        // Add target app bundle ID if provided
        if let Some(app_id) = target_app_bundle_id {
            launch_cmd.env("SIMCTL_CHILD_ARKAVO_TARGET_APP_ID", app_id);
        }

        eprintln!("[XCTestCompiler] Executing launch command...");
        let output = launch_cmd
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to launch test host: {}", e)))?;

        eprintln!(
            "[XCTestCompiler] Launch command completed with status: {}",
            output.status
        );
        if !output.stdout.is_empty() {
            eprintln!(
                "[XCTestCompiler] stdout: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        if !output.stderr.is_empty() {
            eprintln!(
                "[XCTestCompiler] stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Test host launch failed with status: {}. stderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        // Verify the app is running
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Check if test host is running
        let check_cmd = Command::new("xcrun")
            .args(["simctl", "listapps", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list apps: {}", e)))?;

        let apps_output = String::from_utf8_lossy(&check_cmd.stdout);
        if !apps_output.contains("com.arkavo.testhost") {
            eprintln!("[XCTestCompiler] WARNING: Test host app not found in running apps list");
        } else {
            eprintln!("[XCTestCompiler] Test host app confirmed in apps list");
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
