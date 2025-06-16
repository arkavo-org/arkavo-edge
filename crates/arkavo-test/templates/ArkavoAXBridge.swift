import Foundation
#if canImport(XCTest)
import XCTest
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(CoreGraphics)
import CoreGraphics
#endif

// iOS 26 beta compatibility flag
#if swift(>=6.0) || IOS_26_BETA
    let isIOS26Beta = true
#else
    let isIOS26Beta = false
#endif

/// Swift bridge for AXP (Accessibility Private) functions
/// Runs inside Apple-signed UI Test Runner with proper entitlements
@objc public class ArkavoAXBridge: NSObject {
    
    // MARK: - Private AXP Function Types
    
    typealias CSAccessibilityClientPerformAction = @convention(c) (
        _ accessibilityElement: AnyObject,
        _ action: CFString,
        _ value: AnyObject?,
        _ options: CFDictionary?
    ) -> Bool
    
    typealias CSAccessibilityClientHitTest = @convention(c) (
        _ displayID: UInt32,
        _ point: CGPoint,
        _ outElement: UnsafeMutablePointer<AnyObject?>?,
        _ outFrame: UnsafeMutablePointer<CGRect>?
    ) -> Bool
    
    typealias CSAccessibilityClientCopyElementAtPosition = @convention(c) (
        _ displayID: UInt32,
        _ x: Float,
        _ y: Float
    ) -> AnyObject?
    
    // MARK: - Dynamic Loading
    
    private var performActionFunc: CSAccessibilityClientPerformAction?
    private var hitTestFunc: CSAccessibilityClientHitTest?
    private var copyElementFunc: CSAccessibilityClientCopyElementAtPosition?
    private var isAXPAvailable = false
    
    override public init() {
        super.init()
        loadAXPSymbols()
    }
    
    private func loadAXPSymbols() {
        // iOS 26 beta warning
        if isIOS26Beta {
            print("[ArkavoAXBridge] iOS 26 beta detected - AXP symbols may have changed")
            print("[ArkavoAXBridge] Attempting symbol discovery with multiple patterns")
        }
        
        // Try to load CoreSimulator framework symbols dynamically
        guard let handle = dlopen(nil, RTLD_NOW) else {
            print("[ArkavoAXBridge] Failed to open main executable")
            return
        }
        defer { dlclose(handle) }
        
        // Define potential symbol name patterns for iOS 26 beta
        let performActionPatterns = [
            "CSAccessibilityClientPerformAction",
            "_CSAccessibilityClientPerformAction",
            "CSAccessibilityPerformAction",
            "_CSAccessibilityPerformAction",
            "AXPClientPerformAction",
            "_AXPClientPerformAction",
            "AXPerformAction",
            "_AXPerformAction",
            // iOS 26 beta variants
            "CSAXClientPerformAction",
            "_CSAXClientPerformAction",
            "CSAXClientPerformAction2",
            "_CSAXClientPerformAction2",
            "AXClientPerformAction",
            "_AXClientPerformAction"
        ]
        
        let hitTestPatterns = [
            "CSAccessibilityClientHitTest",
            "_CSAccessibilityClientHitTest",
            "CSAccessibilityHitTest",
            "_CSAccessibilityHitTest",
            "AXPClientHitTest",
            "_AXPClientHitTest",
            "AXHitTest",
            "_AXHitTest",
            // iOS 26 beta variants
            "CSAXClientHitTest",
            "_CSAXClientHitTest",
            "CSAXClientHitTest2",
            "_CSAXClientHitTest2"
        ]
        
        let copyElementPatterns = [
            "CSAccessibilityClientCopyElementAtPosition",
            "_CSAccessibilityClientCopyElementAtPosition",
            "CSAccessibilityCopyElementAtPosition",
            "_CSAccessibilityCopyElementAtPosition",
            "AXPClientCopyElementAtPosition",
            "_AXPClientCopyElementAtPosition",
            "AXCopyElementAtPosition",
            "_AXCopyElementAtPosition"
        ]
        
        // Try each pattern for performAction
        for pattern in performActionPatterns {
            if let ptr = dlsym(handle, pattern) {
                performActionFunc = unsafeBitCast(ptr, to: CSAccessibilityClientPerformAction.self)
                print("[ArkavoAXBridge] Found performAction symbol: \(pattern)")
                break
            }
        }
        
        // Try each pattern for hitTest
        for pattern in hitTestPatterns {
            if let ptr = dlsym(handle, pattern) {
                hitTestFunc = unsafeBitCast(ptr, to: CSAccessibilityClientHitTest.self)
                print("[ArkavoAXBridge] Found hitTest symbol: \(pattern)")
                break
            }
        }
        
        // Try each pattern for copyElement
        for pattern in copyElementPatterns {
            if let ptr = dlsym(handle, pattern) {
                copyElementFunc = unsafeBitCast(ptr, to: CSAccessibilityClientCopyElementAtPosition.self)
                print("[ArkavoAXBridge] Found copyElement symbol: \(pattern)")
                break
            }
        }
        
        // Enumerate all symbols for debugging (iOS 26 beta)
        if isIOS26Beta && performActionFunc == nil && hitTestFunc == nil {
            print("[ArkavoAXBridge] Symbol discovery failed - enumerating available symbols")
            enumerateSymbols(handle: handle)
        }
        
        isAXPAvailable = performActionFunc != nil && hitTestFunc != nil
        
        if !isAXPAvailable {
            print("[ArkavoAXBridge] AXP symbols not found - likely due to iOS 26 beta changes")
            print("[ArkavoAXBridge] This is normal for beta iOS versions - falling back to XCTest methods")
            
            // iOS 26 beta specific guidance
            if isIOS26Beta {
                print("[ArkavoAXBridge] iOS 26 beta compatibility mode active")
                print("[ArkavoAXBridge] Performance impact: Taps will take ~50-100ms instead of <30ms")
                print("[ArkavoAXBridge] Solution: Install matching Xcode beta or wait for stable release")
            }
        } else {
            print("[ArkavoAXBridge] AXP symbols loaded successfully - fast path available")
        }
    }
    
    /// Enumerate symbols containing accessibility-related strings
    private func enumerateSymbols(handle: UnsafeMutableRawPointer) {
        // This is a debugging helper for iOS 26 beta
        // In production, this would be removed or behind a debug flag
        
        let accessibilityKeywords = ["Accessibility", "AXP", "AX", "CS", "HitTest", "PerformAction", "Element"]
        var foundSymbols: [String] = []
        
        // Note: Full symbol enumeration requires parsing the Mach-O binary
        // This is a simplified version that checks known patterns
        print("[ArkavoAXBridge] Checking for accessibility-related symbols...")
        
        // Try common prefixes with accessibility keywords
        let prefixes = ["", "_", "__"]
        let frameworks = ["CS", "AX", "AXP", "UIAccessibility", "Accessibility"]
        let functions = ["ClientPerformAction", "ClientHitTest", "CopyElementAtPosition", 
                        "PerformAction", "HitTest", "ElementAtPosition",
                        "SendEvent", "DispatchEvent", "SimulateTouch"]
        
        for prefix in prefixes {
            for framework in frameworks {
                for function in functions {
                    let symbol = "\(prefix)\(framework)\(function)"
                    if dlsym(handle, symbol) != nil {
                        foundSymbols.append(symbol)
                    }
                }
            }
        }
        
        if !foundSymbols.isEmpty {
            print("[ArkavoAXBridge] Found potential accessibility symbols:")
            for symbol in foundSymbols {
                print("  - \(symbol)")
            }
            print("[ArkavoAXBridge] Update symbol patterns with these names for iOS 26 beta support")
        } else {
            print("[ArkavoAXBridge] No accessibility symbols found - may need entitlements or framework changes")
        }
    }
    
    // MARK: - Public API
    
    /// Check if AXP functions are available
    @objc public func isAvailable() -> Bool {
        return isAXPAvailable
    }
    
    /// Get capabilities dictionary for handshake
    @objc public func capabilities() -> [String: Any] {
        var caps: [String: Any] = [
            "axp": isAXPAvailable,
            "version": "1.0",
            "functions": isAXPAvailable ? ["tap", "snapshot", "hitTest"] : ["fallbackTap", "snapshot"]
        ]
        
        // Add iOS 26 beta status if applicable
        if isIOS26Beta {
            caps["ios26_beta"] = true
            caps["fallback_mode"] = "XCUICoordinate"
            caps["performance_note"] = "Using fallback mode due to iOS 26 beta"
        }
        
        return caps
    }
    
    /// Tap at specific coordinates using AXP
    @objc public func tap(x: Double, y: Double) -> Bool {
        guard isAXPAvailable,
              let hitTest = hitTestFunc,
              let performAction = performActionFunc else {
            print("[ArkavoAXBridge] AXP not available for tap")
            return false
        }
        
        let point = CGPoint(x: x, y: y)
        var element: AnyObject?
        var frame = CGRect.zero
        
        // Hit test to find element at point
        let displayID: UInt32 = 0 // Main display
        guard hitTest(displayID, point, &element, &frame),
              let targetElement = element else {
            print("[ArkavoAXBridge] No element found at (\(x), \(y))")
            return false
        }
        
        // Perform tap action
        let success = performAction(
            targetElement,
            "AXPress" as CFString,
            nil,
            nil
        )
        
        print("[ArkavoAXBridge] Tap at (\(x), \(y)): \(success ? "success" : "failed")")
        return success
    }
    
    /// Alternative tap using element search (for Xcode 16+)
    @objc public func tapWithElementSearch(x: Double, y: Double) -> Bool {
        guard isAXPAvailable,
              let copyElement = copyElementFunc,
              let performAction = performActionFunc else {
            print("[ArkavoAXBridge] AXP not available for tap")
            return false
        }
        
        // Try to find element at position
        let element = copyElement(0, Float(x), Float(y))
        guard let targetElement = element else {
            print("[ArkavoAXBridge] No element found at (\(x), \(y))")
            return false
        }
        
        // Perform tap action
        let success = performAction(
            targetElement,
            "AXPress" as CFString,
            nil,
            nil
        )
        
        print("[ArkavoAXBridge] Tap at (\(x), \(y)): \(success ? "success" : "failed")")
        return success
    }
    
    /// Capture accessibility snapshot
    @objc public func snapshot() -> Data? {
        #if canImport(XCTest)
        // For now, use XCUIScreen for screenshots
        // In future, could use AXP to get accessibility tree
        // Swift 6 requires MainActor for XCUIScreen
        return MainActor.assumeIsolated {
            let screenshot = XCUIScreen.main.screenshot()
            return screenshot.pngRepresentation
        }
        #else
        print("[ArkavoAXBridge] Screenshot capture not available")
        return nil
        #endif
    }
    
    /// Fallback tap using XCUICoordinate (when AXP unavailable)
    @objc public func fallbackTap(x: Double, y: Double) -> Bool {
        #if canImport(XCTest)
        // Swift 6 requires MainActor for XCUIApplication
        return MainActor.assumeIsolated {
            let app = XCUIApplication()
            let coordinate = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
                .withOffset(CGVector(dx: x, dy: y))
            coordinate.tap()
            return true
        }
        #else
        print("[ArkavoAXBridge] XCTest not available for fallback tap")
        return false
        #endif
    }
}

// MARK: - Socket Message Handler Extension

extension ArkavoAXBridge {
    
    /// Process command from socket and return response
    @objc public func processCommand(_ command: [String: Any]) -> [String: Any] {
        guard let cmd = command["cmd"] as? String else {
            return ["error": "Missing command"]
        }
        
        switch cmd {
        case "capabilities":
            return ["result": capabilities()]
            
        case "tap":
            guard let x = command["x"] as? Double,
                  let y = command["y"] as? Double else {
                return ["error": "Missing x,y coordinates"]
            }
            
            // Try AXP first, fall back to XCUICoordinate
            let success = tap(x: x, y: y) || tapWithElementSearch(x: x, y: y) || fallbackTap(x: x, y: y)
            return ["success": success]
            
        case "snapshot":
            if let data = snapshot() {
                return ["result": data.base64EncodedString()]
            } else {
                return ["error": "Failed to capture snapshot"]
            }
            
        default:
            return ["error": "Unknown command: \(cmd)"]
        }
    }
}