
/// Enhanced error handling for IDB-related issues
pub struct IdbErrorHandler;

impl IdbErrorHandler {
    /// Analyze an error message and provide actionable guidance
    pub fn analyze_and_guide(error: &str) -> ErrorGuidance {
        let mut guidance = ErrorGuidance {
            error_type: ErrorType::Unknown,
            user_message: String::new(),
            technical_details: error.to_string(),
            suggested_fixes: Vec::new(),
            can_auto_recover: false,
        };
        
        // Port conflict errors
        if error.contains("Address already in use") || error.contains("port 10882") {
            guidance.error_type = ErrorType::PortConflict;
            guidance.user_message = "IDB companion port conflict detected".to_string();
            guidance.suggested_fixes.push("Run: pkill -9 idb_companion".to_string());
            guidance.suggested_fixes.push("Or wait for auto-recovery to clean up stuck processes".to_string());
            guidance.can_auto_recover = true;
        }
        
        // Device not found errors
        else if error.contains("not found in IDB targets") {
            guidance.error_type = ErrorType::DeviceNotFound;
            guidance.user_message = "Device not visible to IDB".to_string();
            guidance.suggested_fixes.push("Ensure the simulator is booted".to_string());
            guidance.suggested_fixes.push("Try: xcrun simctl boot <device-id>".to_string());
            guidance.suggested_fixes.push("Verify device ID is correct".to_string());
            guidance.can_auto_recover = false;
        }
        
        // Connection refused errors
        else if error.contains("Connection refused") || error.contains("failed to connect") {
            guidance.error_type = ErrorType::ConnectionFailed;
            guidance.user_message = "Cannot connect to IDB companion".to_string();
            guidance.suggested_fixes.push("IDB companion may not be running".to_string());
            guidance.suggested_fixes.push("Check if port 10882 is blocked by firewall".to_string());
            guidance.suggested_fixes.push("Try restarting the IDB companion".to_string());
            guidance.can_auto_recover = true;
        }
        
        // Framework conflicts
        else if error.contains("Class FBProcess is implemented in both") {
            guidance.error_type = ErrorType::FrameworkConflict;
            guidance.user_message = "IDB framework conflict detected".to_string();
            guidance.suggested_fixes.push("Install system IDB: brew install facebook/fb/idb-companion".to_string());
            guidance.suggested_fixes.push("Set: export ARKAVO_USE_SYSTEM_IDB=1".to_string());
            guidance.suggested_fixes.push("The system will attempt to use simctl as fallback".to_string());
            guidance.can_auto_recover = false;
        }
        
        // Binary not found
        else if error.contains("ENOENT") || error.contains("No such file") {
            guidance.error_type = ErrorType::BinaryNotFound;
            guidance.user_message = "IDB companion binary not found".to_string();
            guidance.suggested_fixes.push("Rebuild the project: cargo build".to_string());
            guidance.suggested_fixes.push("Or install system IDB: brew install facebook/fb/idb-companion".to_string());
            guidance.can_auto_recover = false;
        }
        
        // Security/signing issues
        else if error.contains("SIGKILL") || error.contains("code signature") {
            guidance.error_type = ErrorType::SecurityBlocked;
            guidance.user_message = "macOS security is blocking IDB companion".to_string();
            guidance.suggested_fixes.push("Add Terminal to Privacy & Security > Developer Tools".to_string());
            guidance.suggested_fixes.push("Or install signed system IDB: brew install facebook/fb/idb-companion".to_string());
            guidance.suggested_fixes.push("Temporary fix: sudo spctl --master-disable (not recommended)".to_string());
            guidance.can_auto_recover = false;
        }
        
        guidance
    }
    
    /// Format error guidance for display
    pub fn format_guidance(guidance: &ErrorGuidance) -> String {
        let mut output = Vec::new();
        
        output.push(format!("\n⚠️  IDB Error: {}", guidance.user_message));
        output.push(format!("Type: {:?}", guidance.error_type));
        
        if !guidance.suggested_fixes.is_empty() {
            output.push("\n📋 Suggested fixes:".to_string());
            for (i, fix) in guidance.suggested_fixes.iter().enumerate() {
                output.push(format!("   {}. {}", i + 1, fix));
            }
        }
        
        if guidance.can_auto_recover {
            output.push("\n✅ Auto-recovery available - the system will attempt to fix this automatically".to_string());
        } else {
            output.push("\n❌ Manual intervention required".to_string());
        }
        
        output.push(format!("\n🔍 Technical details: {}", guidance.technical_details));
        
        output.join("\n")
    }
}

#[derive(Debug)]
pub struct ErrorGuidance {
    pub error_type: ErrorType,
    pub user_message: String,
    pub technical_details: String,
    pub suggested_fixes: Vec<String>,
    pub can_auto_recover: bool,
}

#[derive(Debug, PartialEq)]
pub enum ErrorType {
    PortConflict,
    DeviceNotFound,
    ConnectionFailed,
    FrameworkConflict,
    BinaryNotFound,
    SecurityBlocked,
    Unknown,
}

/// Common IDB troubleshooting guide
pub fn get_troubleshooting_guide() -> String {
    r#"
🔧 IDB Companion Troubleshooting Guide
=====================================

1. Port Conflicts (port 10882 in use)
   - Kill stuck processes: pkill -9 idb_companion
   - Check what's using the port: lsof -i :10882
   - The MCP server will attempt auto-recovery

2. Device Not Found
   - List devices: xcrun simctl list devices
   - Boot device: xcrun simctl boot <device-id>
   - Verify IDB sees it: idb_companion --list 1

3. Connection Issues
   - Check IDB is running: ps aux | grep idb_companion
   - Test connection: nc -zv localhost 10882
   - Restart IDB companion manually if needed

4. Framework Conflicts
   - Install system IDB: brew install facebook/fb/idb-companion
   - Use system IDB: export ARKAVO_USE_SYSTEM_IDB=1
   - The system will fallback to simctl automatically

5. Security/Signing Issues
   - Add your IDE/Terminal to Developer Tools in Privacy & Security
   - Or use system-installed IDB which is properly signed
   - Last resort: sudo spctl --master-disable (not recommended)

For persistent issues:
- Check logs in the MCP server output
- Verify Xcode and simulators are up to date
- Report issues with full error messages
"#.to_string()
}