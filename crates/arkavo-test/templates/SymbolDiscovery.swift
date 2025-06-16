#!/usr/bin/env swift

import Foundation

/// Diagnostic tool to discover AXP symbols in iOS 26 beta
/// Run this on a simulator to find what symbols are actually available

func discoverSymbols() {
    print("=== AXP Symbol Discovery Tool ===")
    print("Searching for accessibility symbols in iOS 26 beta...")
    print("")
    
    // Framework paths to check
    let frameworkPaths = [
        "/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework/AccessibilityPlatformTranslation",
        "/System/Library/PrivateFrameworks/AccessibilityUtilities.framework/AccessibilityUtilities",
        "/System/Library/PrivateFrameworks/AccessibilityUIUtilities.framework/AccessibilityUIUtilities",
        "/System/Library/PrivateFrameworks/AXRuntime.framework/AXRuntime"
    ]
    
    for path in frameworkPaths {
        print("Checking: \(path)")
        
        guard let handle = dlopen(path, RTLD_NOW | RTLD_GLOBAL) else {
            print("  ❌ Could not load framework")
            continue
        }
        defer { dlclose(handle) }
        
        print("  ✅ Framework loaded")
        
        // Use nm to list symbols
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/nm")
        task.arguments = ["-gU", path]
        
        let pipe = Pipe()
        task.standardOutput = pipe
        
        do {
            try task.run()
            task.waitUntilExit()
            
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            if let output = String(data: data, encoding: .utf8) {
                let lines = output.split(separator: "\n")
                let axSymbols = lines.filter { line in
                    line.contains("AX") || line.contains("CS") || line.contains("Accessibility")
                }
                
                if !axSymbols.isEmpty {
                    print("  Found \(axSymbols.count) AX-related symbols:")
                    for symbol in axSymbols.prefix(20) {
                        let parts = symbol.split(separator: " ")
                        if let name = parts.last {
                            print("    - \(name)")
                        }
                    }
                    if axSymbols.count > 20 {
                        print("    ... and \(axSymbols.count - 20) more")
                    }
                }
            }
        } catch {
            print("  ⚠️  Could not run nm: \(error)")
        }
        
        print("")
    }
    
    // Also try to find symbols in the main executable
    print("Checking main executable for linked symbols...")
    if let mainHandle = dlopen(nil, RTLD_NOW) {
        defer { dlclose(mainHandle) }
        
        // Test specific symbols
        let testSymbols = [
            "CSAccessibilityClientPerformAction",
            "_CSAccessibilityClientPerformAction",
            "CSAXClientPerformAction",
            "_CSAXClientPerformAction",
            "CSAccessibilityPerformAction",
            "_CSAccessibilityPerformAction"
        ]
        
        var foundAny = false
        for symbol in testSymbols {
            if dlsym(mainHandle, symbol) != nil {
                print("  ✅ Found: \(symbol)")
                foundAny = true
            }
        }
        
        if !foundAny {
            print("  ❌ None of the common symbols found")
        }
    }
}

// Run the discovery
discoverSymbols()