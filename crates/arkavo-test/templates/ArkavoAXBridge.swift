import Foundation
import XCTest

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
        // Try to load CoreSimulator framework symbols dynamically
        guard let handle = dlopen(nil, RTLD_NOW) else {
            print("[ArkavoAXBridge] Failed to open main executable")
            return
        }
        defer { dlclose(handle) }
        
        // Resolve AXP symbols
        if let performActionPtr = dlsym(handle, "CSAccessibilityClientPerformAction") {
            performActionFunc = unsafeBitCast(performActionPtr, to: CSAccessibilityClientPerformAction.self)
            print("[ArkavoAXBridge] Loaded CSAccessibilityClientPerformAction")
        }
        
        if let hitTestPtr = dlsym(handle, "CSAccessibilityClientHitTest") {
            hitTestFunc = unsafeBitCast(hitTestPtr, to: CSAccessibilityClientHitTest.self)
            print("[ArkavoAXBridge] Loaded CSAccessibilityClientHitTest")
        }
        
        if let copyElementPtr = dlsym(handle, "CSAccessibilityClientCopyElementAtPosition") {
            copyElementFunc = unsafeBitCast(copyElementPtr, to: CSAccessibilityClientCopyElementAtPosition.self)
            print("[ArkavoAXBridge] Loaded CSAccessibilityClientCopyElementAtPosition")
        }
        
        isAXPAvailable = performActionFunc != nil && hitTestFunc != nil
        print("[ArkavoAXBridge] AXP availability: \(isAXPAvailable)")
    }
    
    // MARK: - Public API
    
    /// Check if AXP functions are available
    @objc public func isAvailable() -> Bool {
        return isAXPAvailable
    }
    
    /// Get capabilities dictionary for handshake
    @objc public func capabilities() -> [String: Any] {
        return [
            "axp": isAXPAvailable,
            "version": "1.0",
            "functions": isAXPAvailable ? ["tap", "snapshot", "hitTest"] : []
        ]
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
        // For now, use XCUIScreen for screenshots
        // In future, could use AXP to get accessibility tree
        let screenshot = XCUIScreen.main.screenshot()
        return screenshot.pngRepresentation
    }
    
    /// Fallback tap using XCUICoordinate (when AXP unavailable)
    @objc public func fallbackTap(x: Double, y: Double) -> Bool {
        let app = XCUIApplication()
        let coordinate = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            .withOffset(CGVector(dx: x, dy: y))
        coordinate.tap()
        return true
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