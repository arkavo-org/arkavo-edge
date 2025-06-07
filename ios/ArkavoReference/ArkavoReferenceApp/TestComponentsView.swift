import SwiftUI

struct TestComponentsView: View {
    @Environment(\.dismiss) var dismiss
    @State private var selectedTab = 0
    
    var body: some View {
        NavigationView {
            TabView(selection: $selectedTab) {
                BasicElementsView()
                    .tabItem {
                        Label("Basic", systemImage: "square.grid.2x2")
                    }
                    .tag(0)
                
                FormElementsView()
                    .tabItem {
                        Label("Forms", systemImage: "doc.text")
                    }
                    .tag(1)
                
                InteractionTestView()
                    .tabItem {
                        Label("Interactions", systemImage: "hand.tap")
                    }
                    .tag(2)
                
                CoordinateGridView()
                    .tabItem {
                        Label("Grid", systemImage: "grid")
                    }
                    .tag(3)
            }
            .navigationTitle("Test Components")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Done") {
                        dismiss()
                    }
                    .accessibilityIdentifier("test_components_done")
                }
            }
        }
    }
}

// Basic UI Elements
struct BasicElementsView: View {
    @State private var toggleValue = false
    @State private var sliderValue = 50.0
    @State private var selectedSegment = 0
    @State private var showingAlert = false
    @State private var showingSheet = false
    
    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                // Buttons Section
                GroupBox("Buttons") {
                    VStack(spacing: 12) {
                        Button("Primary Button") { }
                            .buttonStyle(.borderedProminent)
                            .accessibilityIdentifier("primary_button")
                        
                        Button("Secondary Button") { }
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("secondary_button")
                        
                        Button("Destructive Button") { }
                            .buttonStyle(.bordered)
                            .tint(.red)
                            .accessibilityIdentifier("destructive_button")
                        
                        Button("Disabled Button") { }
                            .disabled(true)
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("disabled_button")
                    }
                }
                .accessibilityIdentifier("buttons_group")
                
                // Toggle Section
                GroupBox("Toggles & Switches") {
                    VStack(spacing: 12) {
                        Toggle("Test Toggle", isOn: $toggleValue)
                            .accessibilityIdentifier("test_toggle")
                        
                        HStack {
                            Text("Toggle State:")
                            Text(toggleValue ? "ON" : "OFF")
                                .bold()
                                .foregroundColor(toggleValue ? .green : .red)
                                .accessibilityIdentifier("toggle_state_label")
                        }
                    }
                }
                .accessibilityIdentifier("toggles_group")
                
                // Slider Section
                GroupBox("Slider") {
                    VStack(spacing: 12) {
                        Slider(value: $sliderValue, in: 0...100)
                            .accessibilityIdentifier("test_slider")
                        
                        Text("Value: \(Int(sliderValue))")
                            .accessibilityIdentifier("slider_value_label")
                    }
                }
                .accessibilityIdentifier("slider_group")
                
                // Segmented Control
                GroupBox("Segmented Control") {
                    Picker("Options", selection: $selectedSegment) {
                        Text("First").tag(0)
                        Text("Second").tag(1)
                        Text("Third").tag(2)
                    }
                    .pickerStyle(.segmented)
                    .accessibilityIdentifier("segmented_control")
                }
                .accessibilityIdentifier("segmented_group")
                
                // Alerts & Sheets
                GroupBox("Dialogs") {
                    VStack(spacing: 12) {
                        Button("Show Alert") {
                            showingAlert = true
                        }
                        .accessibilityIdentifier("show_alert_button")
                        
                        Button("Show Sheet") {
                            showingSheet = true
                        }
                        .accessibilityIdentifier("show_sheet_button")
                    }
                }
                .accessibilityIdentifier("dialogs_group")
            }
            .padding()
        }
        .alert("Test Alert", isPresented: $showingAlert) {
            Button("OK") { }
                .accessibilityIdentifier("alert_ok_button")
            Button("Cancel", role: .cancel) { }
                .accessibilityIdentifier("alert_cancel_button")
        } message: {
            Text("This is a test alert message")
                .accessibilityIdentifier("alert_message")
        }
        .sheet(isPresented: $showingSheet) {
            SheetContentView()
        }
    }
}

// Form Elements
struct FormElementsView: View {
    @State private var textFieldValue = ""
    @State private var secureFieldValue = ""
    @State private var textEditorValue = "Multi-line\ntext\nhere"
    @State private var pickerSelection = 0
    @State private var dateSelection = Date()
    @State private var checkboxValues = [false, false, false, false]
    
    var body: some View {
        Form {
            Section("Text Inputs") {
                TextField("Username", text: $textFieldValue)
                    .accessibilityIdentifier("form_username_field")
                
                SecureField("Password", text: $secureFieldValue)
                    .accessibilityIdentifier("form_password_field")
                
                VStack(alignment: .leading) {
                    Text("Notes")
                        .font(.caption)
                    TextEditor(text: $textEditorValue)
                        .frame(height: 100)
                        .accessibilityIdentifier("form_text_editor")
                }
            }
            .accessibilityIdentifier("text_inputs_section")
            
            Section("Pickers") {
                Picker("Select Option", selection: $pickerSelection) {
                    Text("Option 1").tag(0)
                    Text("Option 2").tag(1)
                    Text("Option 3").tag(2)
                }
                .accessibilityIdentifier("form_picker")
                
                DatePicker("Select Date", selection: $dateSelection)
                    .accessibilityIdentifier("form_date_picker")
            }
            .accessibilityIdentifier("pickers_section")
            
            Section("Checkboxes") {
                ForEach(0..<4) { index in
                    HStack {
                        Button(action: {
                            checkboxValues[index].toggle()
                        }) {
                            Image(systemName: checkboxValues[index] ? "checkmark.square" : "square")
                                .foregroundColor(.blue)
                                .accessibilityIdentifier("checkbox_\(index)")
                        }
                        
                        Text("Option \(index + 1)")
                            .accessibilityIdentifier("checkbox_label_\(index)")
                    }
                }
            }
            .accessibilityIdentifier("checkboxes_section")
        }
    }
}

// Interaction Tests
struct InteractionTestView: View {
    @State private var tapCount = 0
    @State private var doubleTapCount = 0
    @State private var longPressCount = 0
    @State private var dragOffset = CGSize.zero
    @State private var lastTapLocation = CGPoint.zero
    @State private var showingTapIndicator = false
    
    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                // Tap Test Area
                GroupBox("Tap Testing") {
                    VStack(spacing: 12) {
                        Rectangle()
                            .fill(Color.blue.opacity(0.3))
                            .frame(height: 150)
                            .overlay(
                                VStack {
                                    Text("Tap anywhere")
                                        .foregroundColor(.blue)
                                    Text("Taps: \(tapCount)")
                                        .accessibilityIdentifier("tap_count_label")
                                }
                            )
                            .onTapGesture { location in
                                tapCount += 1
                                lastTapLocation = location
                                showTapIndicator()
                            }
                            .accessibilityIdentifier("tap_test_area")
                        
                        if showingTapIndicator {
                            Text("Last tap: (\(Int(lastTapLocation.x)), \(Int(lastTapLocation.y)))")
                                .font(.caption)
                                .accessibilityIdentifier("last_tap_location")
                        }
                    }
                }
                .accessibilityIdentifier("tap_testing_group")
                
                // Double Tap Test
                GroupBox("Double Tap Testing") {
                    Rectangle()
                        .fill(Color.green.opacity(0.3))
                        .frame(height: 100)
                        .overlay(
                            Text("Double taps: \(doubleTapCount)")
                                .foregroundColor(.green)
                                .accessibilityIdentifier("double_tap_count_label")
                        )
                        .onTapGesture(count: 2) {
                            doubleTapCount += 1
                        }
                        .accessibilityIdentifier("double_tap_test_area")
                }
                .accessibilityIdentifier("double_tap_group")
                
                // Long Press Test
                GroupBox("Long Press Testing") {
                    Rectangle()
                        .fill(Color.orange.opacity(0.3))
                        .frame(height: 100)
                        .overlay(
                            Text("Long presses: \(longPressCount)")
                                .foregroundColor(.orange)
                                .accessibilityIdentifier("long_press_count_label")
                        )
                        .onLongPressGesture {
                            longPressCount += 1
                        }
                        .accessibilityIdentifier("long_press_test_area")
                }
                .accessibilityIdentifier("long_press_group")
                
                // Drag Test
                GroupBox("Drag Testing") {
                    Rectangle()
                        .fill(Color.purple.opacity(0.3))
                        .frame(width: 100, height: 100)
                        .overlay(
                            Image(systemName: "move.3d")
                                .foregroundColor(.purple)
                        )
                        .offset(dragOffset)
                        .gesture(
                            DragGesture()
                                .onChanged { value in
                                    dragOffset = value.translation
                                }
                                .onEnded { _ in
                                    dragOffset = .zero
                                }
                        )
                        .accessibilityIdentifier("draggable_element")
                }
                .accessibilityIdentifier("drag_test_group")
            }
            .padding()
        }
    }
    
    private func showTapIndicator() {
        showingTapIndicator = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            showingTapIndicator = false
        }
    }
}

// Coordinate Grid View
struct CoordinateGridView: View {
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                // Grid lines
                ForEach(0..<11) { i in
                    // Vertical lines
                    Path { path in
                        let x = geometry.size.width * CGFloat(i) / 10
                        path.move(to: CGPoint(x: x, y: 0))
                        path.addLine(to: CGPoint(x: x, y: geometry.size.height))
                    }
                    .stroke(Color.gray.opacity(0.3), lineWidth: 1)
                    
                    // Horizontal lines
                    Path { path in
                        let y = geometry.size.height * CGFloat(i) / 10
                        path.move(to: CGPoint(x: 0, y: y))
                        path.addLine(to: CGPoint(x: geometry.size.width, y: y))
                    }
                    .stroke(Color.gray.opacity(0.3), lineWidth: 1)
                }
                
                // Percentage labels
                VStack {
                    ForEach(0..<11) { row in
                        HStack {
                            ForEach(0..<11) { col in
                                if row % 2 == 0 && col % 2 == 0 {
                                    Text("\(col*10),\(row*10)")
                                        .font(.system(size: 10))
                                        .foregroundColor(.blue)
                                        .accessibilityIdentifier("grid_label_\(col)_\(row)")
                                }
                                Spacer()
                            }
                        }
                        if row < 10 {
                            Spacer()
                        }
                    }
                }
                .padding(5)
                
                // Test points at specific percentages
                ForEach(testPoints, id: \.id) { point in
                    Circle()
                        .fill(point.color)
                        .frame(width: 20, height: 20)
                        .position(
                            x: geometry.size.width * point.x / 100,
                            y: geometry.size.height * point.y / 100
                        )
                        .accessibilityIdentifier(point.identifier)
                }
            }
            .background(Color.gray.opacity(0.1))
            .accessibilityIdentifier("coordinate_grid")
        }
    }
    
    private var testPoints: [TestPoint] {
        [
            TestPoint(x: 50, y: 50, color: .red, identifier: "center_point"),
            TestPoint(x: 10, y: 10, color: .blue, identifier: "top_left_point"),
            TestPoint(x: 90, y: 10, color: .green, identifier: "top_right_point"),
            TestPoint(x: 10, y: 90, color: .orange, identifier: "bottom_left_point"),
            TestPoint(x: 90, y: 90, color: .purple, identifier: "bottom_right_point"),
        ]
    }
}

struct TestPoint {
    let id = UUID()
    let x: CGFloat
    let y: CGFloat
    let color: Color
    let identifier: String
}

// Sheet Content
struct SheetContentView: View {
    @Environment(\.dismiss) var dismiss
    
    var body: some View {
        NavigationView {
            VStack(spacing: 20) {
                Text("This is a modal sheet")
                    .font(.title)
                    .accessibilityIdentifier("sheet_title")
                
                Text("Sheets can contain any content")
                    .accessibilityIdentifier("sheet_description")
                
                Button("Dismiss") {
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("sheet_dismiss_button")
            }
            .padding()
            .navigationTitle("Sheet")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

#Preview {
    TestComponentsView()
}