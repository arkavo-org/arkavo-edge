import SwiftUI

@main
struct ArkavoReferenceApp: App {
    @StateObject private var navigationManager = NavigationManager()
    @StateObject private var diagnosticManager = DiagnosticManager.shared
    
    var body: some Scene {
        WindowGroup {
            ZStack {
                ContentView()
                    .environmentObject(navigationManager)
                    .environmentObject(diagnosticManager)
                    .onOpenURL { url in
                        handleDeepLink(url)
                    }
                
                // Diagnostic overlay
                GeometryReader { geometry in
                    DiagnosticOverlay(screenSize: geometry.size)
                }
                .ignoresSafeArea()
            }
            .onAppear {
                setupApp()
            }
        }
    }
    
    private func setupApp() {
        // Enable diagnostics in debug mode
        #if DEBUG
        diagnosticManager.isEnabled = true
        #endif
        
        // Log app launch
        diagnosticManager.logEvent(DiagnosticEvent(
            type: .navigation,
            details: "App launched - starting in calibration mode"
        ))
    }
    
    private func handleDeepLink(_ url: URL) {
        // Support both schemes:
        // arkavo-edge://calibration
        // arkavo-reference://screen/calibration
        
        diagnosticManager.logEvent(DiagnosticEvent(
            type: .navigation,
            details: "Deep link: \(url.absoluteString)"
        ))
        
        // Handle simple arkavo-edge:// scheme
        if url.scheme == "arkavo-edge" {
            switch url.host {
            case "calibration":
                navigationManager.navigateToScreen("calibration")
            case "test":
                navigationManager.navigateToScreen("test")
            default:
                break
            }
            return
        }
        
        // Handle arkavo-reference:// scheme
        guard url.scheme == "arkavo-reference" else { return }
        
        switch url.host {
        case "screen":
            if let screenName = url.pathComponents.dropFirst().first {
                navigationManager.navigateToScreen(screenName)
            }
        case "diagnostic":
            if let action = url.pathComponents.dropFirst().first {
                handleDiagnosticAction(action)
            }
        case "test":
            if let testName = url.pathComponents.dropFirst().first {
                runTest(testName)
            }
        default:
            break
        }
    }
    
    private func handleDiagnosticAction(_ action: String) {
        switch action {
        case "enable":
            diagnosticManager.isEnabled = true
        case "disable":
            diagnosticManager.isEnabled = false
        case "clear":
            diagnosticManager.events.removeAll()
        case "export":
            exportDiagnosticData()
        default:
            break
        }
    }
    
    private func runTest(_ testName: String) {
        // Automated test sequences
        Task {
            switch testName {
            case "checkbox_sequence":
                await runCheckboxSequence()
            case "biometric_flow":
                await runBiometricFlow()
            case "form_validation":
                await runFormValidation()
            default:
                break
            }
        }
    }
    
    private func runCheckboxSequence() async {
        // Automated checkbox test sequence
        diagnosticManager.logEvent(DiagnosticEvent(
            type: .stateChange,
            details: "Starting checkbox sequence test"
        ))
        
        // Navigate to checkboxes screen
        navigationManager.navigateToScreen("checkboxes")
        
        // Simulate checkbox interactions
        // This would trigger UI updates that can be observed
    }
    
    private func runBiometricFlow() async {
        // Automated biometric test sequence
        diagnosticManager.logEvent(DiagnosticEvent(
            type: .biometric,
            details: "Starting biometric flow test"
        ))
        
        navigationManager.navigateToScreen("biometric")
    }
    
    private func runFormValidation() async {
        // Automated form validation test
        diagnosticManager.logEvent(DiagnosticEvent(
            type: .stateChange,
            details: "Starting form validation test"
        ))
        
        navigationManager.navigateToScreen("forms")
    }
    
    private func exportDiagnosticData() {
        let data = DiagnosticExporter.export(events: diagnosticManager.events)
        // Save to documents or share
        print("Diagnostic data exported: \(data)")
    }
}

// Navigation Manager
class NavigationManager: ObservableObject {
    @Published var currentScreen: Screen = .main
    @Published var navigationPath = NavigationPath()
    
    enum Screen: String {
        case main
        case checkboxes
        case buttons
        case forms
        case biometric
        case interactions
        case grid
        case edges
        case calibration
        case test
    }
    
    func navigateToScreen(_ screenName: String) {
        if let screen = Screen(rawValue: screenName) {
            currentScreen = screen
            DiagnosticManager.shared.logEvent(DiagnosticEvent(
                type: .navigation,
                details: "Navigated to \(screenName)"
            ))
        }
    }
}

// Diagnostic Data Exporter
struct DiagnosticExporter {
    static func export(events: [DiagnosticEvent]) -> String {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = .prettyPrinted
        
        let exportData = DiagnosticExportData(
            timestamp: Date(),
            deviceInfo: getDeviceInfo(),
            events: events.map { DiagnosticExportEvent(from: $0) }
        )
        
        if let data = try? encoder.encode(exportData),
           let json = String(data: data, encoding: .utf8) {
            return json
        }
        
        return "{}"
    }
    
    static func getDeviceInfo() -> DeviceInfo {
        let device = UIDevice.current
        let screen = UIScreen.main
        
        return DeviceInfo(
            model: device.model,
            systemVersion: device.systemVersion,
            screenSize: "\(Int(screen.bounds.width))×\(Int(screen.bounds.height))",
            screenScale: screen.scale
        )
    }
}

struct DiagnosticExportData: Codable {
    let timestamp: Date
    let deviceInfo: DeviceInfo
    let events: [DiagnosticExportEvent]
}

struct DeviceInfo: Codable {
    let model: String
    let systemVersion: String
    let screenSize: String
    let screenScale: CGFloat
}

struct DiagnosticExportEvent: Codable {
    let timestamp: Date
    let type: String
    let location: CGPoint?
    let identifier: String?
    let details: String?
    
    init(from event: DiagnosticEvent) {
        self.timestamp = event.timestamp
        self.type = String(describing: event.type)
        self.location = event.location
        self.identifier = event.identifier
        self.details = event.details
    }
}