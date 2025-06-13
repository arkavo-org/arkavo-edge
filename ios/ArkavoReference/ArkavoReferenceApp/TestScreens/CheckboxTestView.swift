import SwiftUI

struct CheckboxTestView: View {
    @StateObject private var diagnostics = DiagnosticManager.shared
    @State private var checkboxStates = Array(repeating: false, count: 20)
    @State private var masterCheckbox = false
    
    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                // Header with master checkbox
                GroupBox {
                    HStack {
                        CheckboxView(
                            isChecked: $masterCheckbox,
                            identifier: "master_checkbox",
                            label: "Select All"
                        )
                        .onChange(of: masterCheckbox) { newValue in
                            checkboxStates = Array(repeating: newValue, count: checkboxStates.count)
                            diagnostics.logEvent(DiagnosticEvent(
                                type: .stateChange,
                                identifier: "master_checkbox",
                                details: "Master checkbox: \(newValue)"
                            ))
                        }
                        
                        Spacer()
                        
                        Text("Selected: \(checkboxStates.filter { $0 }.count)")
                            .accessibilityIdentifier("selected_count_label")
                    }
                }
                .accessibilityIdentifier("master_checkbox_group")
                
                // Grid of checkboxes
                GroupBox("Standard Checkboxes") {
                    LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 16) {
                        ForEach(0..<8) { index in
                            CheckboxView(
                                isChecked: $checkboxStates[index],
                                identifier: "checkbox_\(index)",
                                label: "Option \(index + 1)"
                            )
                        }
                    }
                }
                .accessibilityIdentifier("standard_checkboxes_group")
                
                // Different checkbox styles
                GroupBox("Checkbox Variations") {
                    VStack(alignment: .leading, spacing: 12) {
                        // Disabled checkbox
                        CheckboxView(
                            isChecked: .constant(true),
                            identifier: "disabled_checkbox_checked",
                            label: "Disabled (Checked)",
                            isDisabled: true
                        )
                        
                        CheckboxView(
                            isChecked: .constant(false),
                            identifier: "disabled_checkbox_unchecked",
                            label: "Disabled (Unchecked)",
                            isDisabled: true
                        )
                        
                        // Custom styled checkbox
                        CheckboxView(
                            isChecked: $checkboxStates[8],
                            identifier: "custom_style_checkbox",
                            label: "Custom Style",
                            style: .custom
                        )
                        
                        // Checkbox with long label
                        CheckboxView(
                            isChecked: $checkboxStates[9],
                            identifier: "long_label_checkbox",
                            label: "This is a checkbox with a very long label that might wrap to multiple lines on smaller devices"
                        )
                    }
                }
                .accessibilityIdentifier("checkbox_variations_group")
                
                // Hidden checkboxes (require scrolling)
                GroupBox("Checkboxes Below Fold") {
                    VStack(alignment: .leading, spacing: 12) {
                        ForEach(10..<20) { index in
                            CheckboxView(
                                isChecked: $checkboxStates[index],
                                identifier: "hidden_checkbox_\(index)",
                                label: "Hidden Option \(index - 9)"
                            )
                        }
                    }
                }
                .accessibilityIdentifier("hidden_checkboxes_group")
                
                // Test result display
                GroupBox("Checkbox States") {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(0..<checkboxStates.count, id: \.self) { index in
                            if checkboxStates[index] {
                                Text("✓ Checkbox \(index) is checked")
                                    .font(.caption)
                                    .foregroundColor(.green)
                                    .accessibilityIdentifier("state_display_\(index)")
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .accessibilityIdentifier("checkbox_states_display")
            }
            .padding()
        }
        .navigationTitle("Checkbox Tests")
        .navigationBarTitleDisplayMode(.inline)
    }
}

// Reusable Checkbox Component
struct CheckboxView: View {
    @Binding var isChecked: Bool
    let identifier: String
    let label: String
    var isDisabled: Bool = false
    var style: CheckboxStyle = .standard
    
    @StateObject private var diagnostics = DiagnosticManager.shared
    
    enum CheckboxStyle {
        case standard
        case custom
    }
    
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Button(action: {
                if !isDisabled {
                    isChecked.toggle()
                    diagnostics.logEvent(DiagnosticEvent(
                        type: .stateChange,
                        identifier: identifier,
                        details: "Checkbox toggled: \(isChecked)"
                    ))
                }
            }) {
                ZStack {
                    // Checkbox background
                    RoundedRectangle(cornerRadius: style == .custom ? 8 : 4)
                        .fill(checkboxBackgroundColor)
                        .frame(width: 24, height: 24)
                    
                    // Checkbox border
                    RoundedRectangle(cornerRadius: style == .custom ? 8 : 4)
                        .stroke(checkboxBorderColor, lineWidth: 2)
                        .frame(width: 24, height: 24)
                    
                    // Checkmark
                    if isChecked {
                        Image(systemName: style == .custom ? "checkmark.circle.fill" : "checkmark")
                            .foregroundColor(checkmarkColor)
                            .font(.system(size: style == .custom ? 20 : 16, weight: .bold))
                    }
                }
            }
            .disabled(isDisabled)
            .accessibilityIdentifier(identifier)
            .accessibilityLabel("\(label) checkbox")
            .accessibilityValue(isChecked ? "checked" : "unchecked")
            .accessibilityHint(isDisabled ? "disabled" : "double tap to toggle")
            
            Text(label)
                .foregroundColor(isDisabled ? .gray : .primary)
                .accessibilityIdentifier("\(identifier)_label")
            
            Spacer()
        }
    }
    
    private var checkboxBackgroundColor: Color {
        if isDisabled {
            return Color.gray.opacity(0.1)
        }
        return isChecked ? Color.blue.opacity(0.1) : Color.clear
    }
    
    private var checkboxBorderColor: Color {
        if isDisabled {
            return Color.gray.opacity(0.5)
        }
        return isChecked ? Color.blue : Color.gray
    }
    
    private var checkmarkColor: Color {
        if isDisabled {
            return Color.gray
        }
        return style == .custom ? Color.blue : Color.white
    }
}

// Test Automation Helper
extension CheckboxTestView {
    static func automatedTest() async {
        let diagnostics = DiagnosticManager.shared
        
        diagnostics.logEvent(DiagnosticEvent(
            type: .stateChange,
            details: "Starting automated checkbox test"
        ))
        
        // Test sequence:
        // 1. Check first 3 checkboxes
        // 2. Toggle master checkbox
        // 3. Uncheck all
        // 4. Check hidden checkboxes
        
        // This would be triggered by deep link or test runner
    }
}
