import SwiftUI

struct CalibrationView: View {
    @State private var tapLog: [TapEvent] = []
    @State private var currentTarget: CalibrationTarget?
    @State private var isCalibrating = false
    @State private var tappedTargets: Set<String> = []
    @State private var lastTapLocation: CGPoint? = nil
    @State private var showTapIndicator = false
    @State private var tapHistory: [TapHistoryItem] = []
    @Environment(\.scenePhase) var scenePhase
    
    let gridSize = 5
    let targetSize: CGFloat = 60
    
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                // Background - capture ALL taps
                Color.white
                    .contentShape(Rectangle())
                    .onTapGesture { location in
                        handleBackgroundTap(at: location, in: geometry.size)
                    }
                
                // Calibration targets
                ForEach(calibrationTargets(in: geometry.size)) { target in
                    CalibrationTargetView(
                        target: target,
                        isActive: currentTarget?.id == target.id,
                        isTapped: tappedTargets.contains(target.id),
                        onTap: { actualLocation in
                            recordTapResult(target: target, actualLocation: actualLocation)
                        }
                    )
                }
                
                // Show persistent tap history
                ForEach(tapHistory) { item in
                    TapHistoryIndicator(item: item)
                }
                
                // Don't show full diagnostic overlay to avoid clutter
                // DiagnosticOverlay(screenSize: geometry.size)
                
                // Status overlay
                VStack {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Calibration Mode")
                                .font(.system(size: 14, weight: .semibold))
                            Text("Tapped: \(tappedTargets.count)/5")
                                .font(.system(size: 12))
                        }
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .background(Color.black.opacity(0.8))
                            .foregroundColor(.white)
                            .cornerRadius(8)
                        
                        Spacer()
                        
                        // Remove last tap info from top to reduce clutter
                    }
                    .padding()
                    
                    Spacer()
                    
                    // Tap history log at bottom
                    VStack(alignment: .leading, spacing: 2) {
                        HStack {
                            Text("Tap History:")
                                .font(.system(size: 12, weight: .semibold))
                            Spacer()
                            Button("Clear") {
                                clearTapHistory()
                            }
                            .font(.system(size: 10))
                            .foregroundColor(.red)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 2)
                            .background(Color.white.opacity(0.2))
                            .cornerRadius(4)
                        }
                        ScrollView {
                            VStack(alignment: .leading, spacing: 1) {
                                ForEach(tapHistory.suffix(5).reversed()) { item in
                                    Text("[\(item.index)] (\(Int(item.location.x)), \(Int(item.location.y))) - \(item.hit ? "HIT" : "MISS")")
                                        .font(.system(size: 10, design: .monospaced))
                                        .foregroundColor(item.hit ? .green : .yellow)
                                }
                            }
                        }
                        .frame(maxHeight: 60)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(Color.black.opacity(0.8))
                    .foregroundColor(.white)
                    .cornerRadius(8)
                    .padding(.horizontal)
                    .padding(.bottom, 40)
                }
                
                // Grid overlay for debugging
                GridOverlay(size: geometry.size)
                    .opacity(0.2)
                
                // Target coordinate labels
                ForEach(calibrationTargets(in: geometry.size)) { target in
                    Text("(\(Int(target.expectedLocation.x)), \(Int(target.expectedLocation.y)))")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.blue)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                        .background(Color.white.opacity(0.9))
                        .cornerRadius(4)
                        .position(x: target.expectedLocation.x, y: target.expectedLocation.y + 35)
                }
            }
        }
        .onAppear {
            startCalibrationServer()
            let size = UIScreen.main.bounds.size
            print("CalibrationView: View appeared with screen size: \(size)")
            print("CalibrationView: Expecting 5 taps at percentage-based positions:")
            let percentagePoints = [
                (0.2, 0.2),   // 20%, 20%
                (0.8, 0.2),   // 80%, 20%
                (0.5, 0.5),   // 50%, 50%
                (0.2, 0.8),   // 20%, 80%
                (0.8, 0.8),   // 80%, 80%
            ]
            for (idx, pct) in percentagePoints.enumerated() {
                let x = size.width * pct.0
                let y = size.height * pct.1
                print("  Target \(idx): (\(x), \(y)) - \(Int(pct.0 * 100))%, \(Int(pct.1 * 100))%")
            }
        }
        .onDisappear {
            stopCalibrationServer()
            // Make sure diagnostics are disabled when leaving
            DiagnosticManager.shared.isEnabled = false
        }
        .onChange(of: scenePhase) { newPhase in
            switch newPhase {
            case .background:
                // Clear tap history when app is minimized
                clearTapHistory()
            case .inactive:
                // Optionally clear on inactive too
                clearTapHistory()
            case .active:
                // App is active again
                print("CalibrationView: App became active")
            @unknown default:
                break
            }
        }
    }
    
    func calibrationTargets(in size: CGSize) -> [CalibrationTarget] {
        var targets: [CalibrationTarget] = []
        
        // Create a grid of targets using percentage-based positioning
        // This ensures consistent positioning across all device sizes
        let positions = [
            (0.2, 0.2),   // 20%, 20%
            (0.8, 0.2),   // 80%, 20%
            (0.5, 0.5),   // 50%, 50%
            (0.2, 0.8),   // 20%, 80%
            (0.8, 0.8),   // 80%, 80%
        ]
        
        for (index, (xPercent, yPercent)) in positions.enumerated() {
            let x = size.width * xPercent
            let y = size.height * yPercent
            
            targets.append(CalibrationTarget(
                id: "target_\(index)",
                expectedLocation: CGPoint(x: x, y: y),
                tolerance: 10.0
            ))
        }
        
        return targets
    }
    
    func handleBackgroundTap(at location: CGPoint, in size: CGSize) {
        // Record ALL taps, whether they hit targets or not
        print("CalibrationView: Tap detected at (\(location.x), \(location.y))")
        
        // Add to tap history
        let tapIndex = tapHistory.count + 1
        let historyItem = TapHistoryItem(
            id: UUID(),
            index: tapIndex,
            location: location,
            hit: false,
            timestamp: Date()
        )
        tapHistory.append(historyItem)
        
        // Don't record in diagnostic manager to avoid duplicates
        
        // Check if this tap is near any target
        var hitTarget: CalibrationTarget? = nil
        for target in calibrationTargets(in: size) {
            let distance = sqrt(pow(location.x - target.expectedLocation.x, 2) + 
                              pow(location.y - target.expectedLocation.y, 2))
            if distance <= 30 { // Within 30 points of target center
                hitTarget = target
                break
            }
        }
        
        if let target = hitTarget {
            // Record as target hit
            recordTapResult(target: target, actualLocation: location)
        } else {
            // Record as background tap
            let event = TapEvent(
                timestamp: Date(),
                expected: CGPoint(x: -1, y: -1),
                actual: location,
                targetHit: false
            )
            tapLog.append(event)
            writeCalibrationData()
        }
    }
    
    func recordTapResult(target: CalibrationTarget, actualLocation: CGPoint) {
        print("CalibrationView: Target \(target.id) tapped at (\(actualLocation.x), \(actualLocation.y))")
        let event = TapEvent(
            timestamp: Date(),
            expected: target.expectedLocation,
            actual: actualLocation,
            targetHit: true
        )
        tapLog.append(event)
        currentTarget = nil
        tappedTargets.insert(target.id)
        
        // Update the last tap in history to show it was a hit
        if let lastIndex = tapHistory.indices.last(where: { tapHistory[$0].location == actualLocation }) {
            tapHistory[lastIndex].hit = true
        }
        
        // Don't record in diagnostics to avoid duplicates
        
        writeCalibrationData()
    }
    
    func writeCalibrationData() {
        // Write to shared container for MCP to read
        let data = CalibrationData(
            deviceInfo: UIDevice.current.name,
            screenSize: UIScreen.main.bounds.size,
            tapEvents: tapLog,
            calibrationComplete: tappedTargets.count >= 5
        )
        
        if let encoded = try? JSONEncoder().encode(data) {
            let url = FileManager.default
                .containerURL(forSecurityApplicationGroupIdentifier: "group.arkavo.calibration")?
                .appendingPathComponent("calibration_results.json")
            
            if let url = url {
                do {
                    try encoded.write(to: url)
                    print("CalibrationView: Wrote results to shared container: \(url)")
                } catch {
                    print("CalibrationView: Failed to write to shared container: \(error)")
                }
            }
            
            // Also write to documents for easier access
            if let documentsPath = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first {
                let fileURL = documentsPath.appendingPathComponent("calibration_results.json")
                do {
                    try encoded.write(to: fileURL)
                    print("CalibrationView: Wrote results to documents: \(fileURL)")
                    print("CalibrationView: Total taps recorded: \(tapLog.count), Targets hit: \(tappedTargets.count)")
                } catch {
                    print("CalibrationView: Failed to write to documents: \(error)")
                }
            }
        }
    }
    
    func startCalibrationServer() {
        // In production, this would start a local HTTP server
        // For now, we use file-based communication
        isCalibrating = true
    }
    
    func stopCalibrationServer() {
        isCalibrating = false
    }
    
    func clearTapHistory() {
        tapHistory.removeAll()
        tapLog.removeAll()
        tappedTargets.removeAll()
        lastTapLocation = nil
        currentTarget = nil
        
        // Clear any persisted calibration data
        if let documentsPath = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first {
            let fileURL = documentsPath.appendingPathComponent("calibration_results.json")
            try? FileManager.default.removeItem(at: fileURL)
        }
        
        print("CalibrationView: Cleared all tap history and calibration data")
    }
}

struct CalibrationTargetView: View {
    let target: CalibrationTarget
    let isActive: Bool
    let isTapped: Bool
    let onTap: (CGPoint) -> Void
    
    var body: some View {
        Circle()
            .fill(isTapped ? Color.green.opacity(0.6) : (isActive ? Color.yellow.opacity(0.6) : Color.blue.opacity(0.3)))
            .frame(width: 60, height: 60)
            .overlay(
                Circle()
                    .stroke(isTapped ? Color.green : Color.blue, lineWidth: 2)
            )
            .overlay(
                Image(systemName: isTapped ? "checkmark" : "plus")
                    .font(.system(size: 30, weight: .regular))
                    .foregroundColor(isTapped ? .green : .blue)
            )
            .position(target.expectedLocation)
            // Remove tap gesture - let background handle all taps for accurate location
            .accessibilityIdentifier(target.id)
            .animation(.easeInOut(duration: 0.3), value: isTapped)
    }
}

struct CalibrationTarget: Identifiable {
    let id: String
    let expectedLocation: CGPoint
    let tolerance: CGFloat
}

struct TapEvent: Codable {
    let timestamp: Date
    let expected: CGPoint
    let actual: CGPoint
    let targetHit: Bool
    
    var offset: CGPoint {
        CGPoint(x: actual.x - expected.x, y: actual.y - expected.y)
    }
}

struct CalibrationData: Codable {
    let deviceInfo: String
    let screenSize: CGSize
    let tapEvents: [TapEvent]
    let calibrationComplete: Bool
    
    var averageOffset: CGPoint {
        guard !tapEvents.isEmpty else { return .zero }
        
        let validTaps = tapEvents.filter { $0.targetHit }
        guard !validTaps.isEmpty else { return .zero }
        
        let sumX = validTaps.reduce(0) { $0 + $1.offset.x }
        let sumY = validTaps.reduce(0) { $0 + $1.offset.y }
        
        return CGPoint(x: sumX / CGFloat(validTaps.count),
                      y: sumY / CGFloat(validTaps.count))
    }
}

// Make CGPoint Codable
extension CGPoint: Codable {
    enum CodingKeys: String, CodingKey {
        case x, y
    }
    
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let x = try container.decode(CGFloat.self, forKey: .x)
        let y = try container.decode(CGFloat.self, forKey: .y)
        self.init(x: x, y: y)
    }
    
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(x, forKey: .x)
        try container.encode(y, forKey: .y)
    }
}

// Make CGSize Codable
extension CGSize: Codable {
    enum CodingKeys: String, CodingKey {
        case width, height
    }
    
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let width = try container.decode(CGFloat.self, forKey: .width)
        let height = try container.decode(CGFloat.self, forKey: .height)
        self.init(width: width, height: height)
    }
    
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(width, forKey: .width)
        try container.encode(height, forKey: .height)
    }
}

// Tap history item for persistent display
struct TapHistoryItem: Identifiable {
    let id: UUID
    let index: Int
    let location: CGPoint
    var hit: Bool
    let timestamp: Date
}

// Tap history indicator view
struct TapHistoryIndicator: View {
    let item: TapHistoryItem
    
    var body: some View {
        ZStack {
            // Outer circle
            Circle()
                .stroke(item.hit ? Color.green : Color.red, lineWidth: 2)
                .frame(width: 20, height: 20)
            
            // Inner dot
            Circle()
                .fill(item.hit ? Color.green.opacity(0.5) : Color.red.opacity(0.5))
                .frame(width: 8, height: 8)
            
            // Tap index
            Text("\(item.index)")
                .font(.system(size: 12, weight: .bold))
                .foregroundColor(.white)
                .padding(4)
                .background(item.hit ? Color.green : Color.red)
                .clipShape(Circle())
                .offset(y: -25)
        }
        .position(item.location)
    }
}