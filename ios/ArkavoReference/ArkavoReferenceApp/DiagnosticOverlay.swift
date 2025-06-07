import SwiftUI

// Global diagnostic state
class DiagnosticManager: ObservableObject {
    static let shared = DiagnosticManager()
    
    @Published var isEnabled = false
    @Published var showGrid = true
    @Published var showTapIndicators = true
    @Published var showCoordinates = true
    @Published var showEventLog = true
    @Published var events: [DiagnosticEvent] = []
    @Published var lastTapLocation: CGPoint? = nil
    @Published var lastTapTime: Date? = nil
    
    func logEvent(_ event: DiagnosticEvent) {
        DispatchQueue.main.async {
            self.events.insert(event, at: 0)
            if self.events.count > 100 {
                self.events.removeLast()
            }
        }
    }
    
    func recordTap(at location: CGPoint, identifier: String? = nil) {
        lastTapLocation = location
        lastTapTime = Date()
        
        logEvent(DiagnosticEvent(
            type: .tap,
            location: location,
            identifier: identifier,
            timestamp: Date()
        ))
    }
}

struct DiagnosticEvent: Identifiable {
    let id = UUID()
    let type: EventType
    let location: CGPoint?
    let identifier: String?
    let timestamp: Date
    let details: String?
    
    init(type: EventType, location: CGPoint? = nil, identifier: String? = nil, details: String? = nil, timestamp: Date = Date()) {
        self.type = type
        self.location = location
        self.identifier = identifier
        self.details = details
        self.timestamp = timestamp
    }
    
    enum EventType {
        case tap
        case stateChange
        case dialogShow
        case dialogDismiss
        case biometric
        case navigation
    }
    
    var description: String {
        let timeFormatter = DateFormatter()
        timeFormatter.dateFormat = "HH:mm:ss.SSS"
        let timeString = timeFormatter.string(from: timestamp)
        
        switch type {
        case .tap:
            if let loc = location {
                return "[\(timeString)] TAP at (\(Int(loc.x)), \(Int(loc.y)))\(identifier.map { " - \($0)" } ?? "")"
            }
            return "[\(timeString)] TAP\(identifier.map { " - \($0)" } ?? "")"
        case .stateChange:
            return "[\(timeString)] STATE: \(details ?? "changed")"
        case .dialogShow:
            return "[\(timeString)] DIALOG SHOWN: \(identifier ?? "unknown")"
        case .dialogDismiss:
            return "[\(timeString)] DIALOG DISMISSED: \(identifier ?? "unknown")"
        case .biometric:
            return "[\(timeString)] BIOMETRIC: \(details ?? "triggered")"
        case .navigation:
            return "[\(timeString)] NAV: \(details ?? "changed")"
        }
    }
}

// Diagnostic Overlay View
struct DiagnosticOverlay: View {
    @StateObject private var diagnostics = DiagnosticManager.shared
    let screenSize: CGSize
    
    var body: some View {
        if diagnostics.isEnabled {
            ZStack {
                // Grid overlay
                if diagnostics.showGrid {
                    GridOverlay(size: screenSize)
                }
                
                // Tap indicator
                if diagnostics.showTapIndicators, let tapLocation = diagnostics.lastTapLocation {
                    TapIndicator(location: tapLocation)
                }
                
                // Coordinate display
                if diagnostics.showCoordinates {
                    CoordinateDisplay(screenSize: screenSize)
                }
                
                // Event log
                if diagnostics.showEventLog {
                    EventLogOverlay()
                }
            }
            .allowsHitTesting(false)
        }
    }
}

// Grid Overlay
struct GridOverlay: View {
    let size: CGSize
    let gridDivisions = 10
    
    var body: some View {
        ZStack {
            // Vertical lines
            ForEach(0...gridDivisions, id: \.self) { i in
                Path { path in
                    let x = size.width * CGFloat(i) / CGFloat(gridDivisions)
                    path.move(to: CGPoint(x: x, y: 0))
                    path.addLine(to: CGPoint(x: x, y: size.height))
                }
                .stroke(Color.blue.opacity(0.2), lineWidth: 0.5)
            }
            
            // Horizontal lines
            ForEach(0...gridDivisions, id: \.self) { i in
                Path { path in
                    let y = size.height * CGFloat(i) / CGFloat(gridDivisions)
                    path.move(to: CGPoint(x: 0, y: y))
                    path.addLine(to: CGPoint(x: size.width, y: y))
                }
                .stroke(Color.blue.opacity(0.2), lineWidth: 0.5)
            }
            
            // Percentage labels at intersections
            ForEach(0...gridDivisions, id: \.self) { row in
                ForEach(0...gridDivisions, id: \.self) { col in
                    if row % 2 == 0 && col % 2 == 0 {
                        Text("\(col*10)%,\(row*10)%")
                            .font(.system(size: 8))
                            .foregroundColor(.blue.opacity(0.6))
                            .position(
                                x: size.width * CGFloat(col) / CGFloat(gridDivisions),
                                y: size.height * CGFloat(row) / CGFloat(gridDivisions)
                            )
                    }
                }
            }
        }
    }
}

// Tap Indicator
struct TapIndicator: View {
    let location: CGPoint
    @State private var scale: CGFloat = 0.5
    @State private var opacity: Double = 1.0
    
    var body: some View {
        ZStack {
            Circle()
                .stroke(Color.red, lineWidth: 3)
                .frame(width: 40, height: 40)
                .scaleEffect(scale)
                .opacity(opacity)
            
            Circle()
                .fill(Color.red.opacity(0.3))
                .frame(width: 20, height: 20)
                .opacity(opacity)
            
            VStack(spacing: 2) {
                Text("(\(Int(location.x)), \(Int(location.y)))")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(.red)
                    .padding(4)
                    .background(Color.white.opacity(0.9))
                    .cornerRadius(4)
            }
            .offset(y: -30)
            .opacity(opacity)
        }
        .position(location)
        .onAppear {
            withAnimation(.easeOut(duration: 0.3)) {
                scale = 1.5
            }
            withAnimation(.easeOut(duration: 1.0).delay(0.3)) {
                opacity = 0
            }
        }
    }
}

// Coordinate Display
struct CoordinateDisplay: View {
    let screenSize: CGSize
    @State private var currentLocation = CGPoint.zero
    
    var body: some View {
        VStack {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Screen: \(Int(screenSize.width)) × \(Int(screenSize.height))")
                    if let lastTap = DiagnosticManager.shared.lastTapLocation {
                        Text("Last tap: (\(Int(lastTap.x)), \(Int(lastTap.y)))")
                        Text("Normalized: (\(String(format: "%.2f", lastTap.x/screenSize.width)), \(String(format: "%.2f", lastTap.y/screenSize.height)))")
                    }
                }
                .font(.system(size: 10, design: .monospaced))
                .padding(8)
                .background(Color.black.opacity(0.8))
                .foregroundColor(.green)
                .cornerRadius(6)
                
                Spacer()
            }
            
            Spacer()
        }
        .padding()
    }
}

// Event Log Overlay
struct EventLogOverlay: View {
    @StateObject private var diagnostics = DiagnosticManager.shared
    
    var body: some View {
        VStack {
            Spacer()
            
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(diagnostics.events.prefix(10)) { event in
                        Text(event.description)
                            .font(.system(size: 9, design: .monospaced))
                            .foregroundColor(.white)
                            .lineLimit(1)
                    }
                }
                .padding(8)
            }
            .frame(maxHeight: 150)
            .background(Color.black.opacity(0.8))
            .cornerRadius(8)
            .padding(.horizontal)
            .padding(.bottom, 50)
        }
    }
}

// Diagnostic-aware button modifier
struct DiagnosticTapModifier: ViewModifier {
    let identifier: String
    
    func body(content: Content) -> some View {
        content
            .onTapGesture { location in
                DiagnosticManager.shared.recordTap(at: location, identifier: identifier)
            }
    }
}

extension View {
    func diagnosticTap(_ identifier: String) -> some View {
        self.modifier(DiagnosticTapModifier(identifier: identifier))
    }
}

// Safe Area Markers
struct SafeAreaMarkers: View {
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                // Top safe area
                Rectangle()
                    .fill(Color.green.opacity(0.2))
                    .frame(height: geometry.safeAreaInsets.top)
                    .position(x: geometry.size.width/2, y: geometry.safeAreaInsets.top/2)
                    .overlay(
                        Text("Safe Area Top: \(Int(geometry.safeAreaInsets.top))pt")
                            .font(.caption2)
                            .foregroundColor(.green)
                    )
                
                // Bottom safe area
                Rectangle()
                    .fill(Color.green.opacity(0.2))
                    .frame(height: geometry.safeAreaInsets.bottom)
                    .position(x: geometry.size.width/2, y: geometry.size.height - geometry.safeAreaInsets.bottom/2)
                    .overlay(
                        Text("Safe Area Bottom: \(Int(geometry.safeAreaInsets.bottom))pt")
                            .font(.caption2)
                            .foregroundColor(.green)
                    )
                
                // Edge markers
                ForEach(edgeMarkers, id: \.id) { marker in
                    Circle()
                        .fill(marker.color)
                        .frame(width: 20, height: 20)
                        .position(
                            x: geometry.size.width * marker.x,
                            y: geometry.size.height * marker.y
                        )
                        .overlay(
                            Text(marker.label)
                                .font(.system(size: 8))
                                .foregroundColor(.white)
                        )
                        .accessibilityIdentifier(marker.identifier)
                }
            }
        }
        .ignoresSafeArea()
    }
    
    private var edgeMarkers: [EdgeMarker] {
        [
            EdgeMarker(x: 0, y: 0, label: "TL", color: .red, identifier: "edge_top_left"),
            EdgeMarker(x: 1, y: 0, label: "TR", color: .blue, identifier: "edge_top_right"),
            EdgeMarker(x: 0, y: 1, label: "BL", color: .green, identifier: "edge_bottom_left"),
            EdgeMarker(x: 1, y: 1, label: "BR", color: .orange, identifier: "edge_bottom_right"),
            EdgeMarker(x: 0.5, y: 0, label: "T", color: .purple, identifier: "edge_top_center"),
            EdgeMarker(x: 0.5, y: 1, label: "B", color: .purple, identifier: "edge_bottom_center"),
            EdgeMarker(x: 0, y: 0.5, label: "L", color: .purple, identifier: "edge_left_center"),
            EdgeMarker(x: 1, y: 0.5, label: "R", color: .purple, identifier: "edge_right_center"),
        ]
    }
}

struct EdgeMarker {
    let id = UUID()
    let x: CGFloat
    let y: CGFloat
    let label: String
    let color: Color
    let identifier: String
}