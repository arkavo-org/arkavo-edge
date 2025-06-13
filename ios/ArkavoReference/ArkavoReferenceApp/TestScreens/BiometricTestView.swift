import SwiftUI
import LocalAuthentication

struct BiometricTestView: View {
    @StateObject private var diagnostics = DiagnosticManager.shared
    @State private var biometricType = LAContext().biometryType
    @State private var testResults: [BiometricTestResult] = []
    @State private var isRunningTest = false
    @State private var showingEnrollmentAlert = false
    @State private var showingAuthenticationPrompt = false
    
    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                // Biometric Status
                GroupBox("Biometric Status") {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack {
                            Text("Type:")
                            Text(biometricTypeString)
                                .bold()
                                .accessibilityIdentifier("biometric_type_label")
                        }
                        
                        HStack {
                            Text("Available:")
                            Text(isBiometricAvailable ? "Yes" : "No")
                                .bold()
                                .foregroundColor(isBiometricAvailable ? .green : .red)
                                .accessibilityIdentifier("biometric_available_label")
                        }
                        
                        HStack {
                            Text("Enrolled:")
                            Text(isBiometricEnrolled ? "Yes" : "No")
                                .bold()
                                .foregroundColor(isBiometricEnrolled ? .green : .orange)
                                .accessibilityIdentifier("biometric_enrolled_label")
                        }
                    }
                }
                .accessibilityIdentifier("biometric_status_group")
                
                // Test Scenarios
                GroupBox("Test Scenarios") {
                    VStack(spacing: 12) {
                        // Success test
                        Button("Test Success Authentication") {
                            testBiometricSuccess()
                        }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("test_biometric_success_button")
                        
                        // Failure test
                        Button("Test Failed Authentication") {
                            testBiometricFailure()
                        }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("test_biometric_failure_button")
                        
                        // Cancel test
                        Button("Test User Cancel") {
                            testBiometricCancel()
                        }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("test_biometric_cancel_button")
                        
                        // Lockout test
                        Button("Test Lockout") {
                            testBiometricLockout()
                        }
                        .buttonStyle(.bordered)
                        .tint(.orange)
                        .accessibilityIdentifier("test_biometric_lockout_button")
                        
                        // Enrollment test
                        Button("Test Not Enrolled") {
                            testBiometricNotEnrolled()
                        }
                        .buttonStyle(.bordered)
                        .tint(.purple)
                        .accessibilityIdentifier("test_biometric_not_enrolled_button")
                        
                        // Passcode fallback
                        Button("Test Passcode Fallback") {
                            testPasscodeFallback()
                        }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("test_passcode_fallback_button")
                    }
                }
                .accessibilityIdentifier("test_scenarios_group")
                
                // Automated Test Runner
                GroupBox("Automated Testing") {
                    VStack(spacing: 12) {
                        Button(isRunningTest ? "Running Tests..." : "Run All Tests") {
                            runAllTests()
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(isRunningTest)
                        .accessibilityIdentifier("run_all_tests_button")
                        
                        if isRunningTest {
                            ProgressView()
                                .accessibilityIdentifier("test_progress_indicator")
                        }
                    }
                }
                .accessibilityIdentifier("automated_testing_group")
                
                // Test Results
                if !testResults.isEmpty {
                    GroupBox("Test Results") {
                        VStack(alignment: .leading, spacing: 8) {
                            ForEach(testResults) { result in
                                HStack {
                                    Image(systemName: result.success ? "checkmark.circle.fill" : "xmark.circle.fill")
                                        .foregroundColor(result.success ? .green : .red)
                                    
                                    VStack(alignment: .leading) {
                                        Text(result.scenario)
                                            .font(.caption)
                                            .bold()
                                        Text(result.message)
                                            .font(.caption2)
                                            .foregroundColor(.secondary)
                                    }
                                    
                                    Spacer()
                                }
                                .accessibilityIdentifier("test_result_\(result.id)")
                            }
                        }
                    }
                    .accessibilityIdentifier("test_results_group")
                }
            }
            .padding()
        }
        .navigationTitle("Biometric Tests")
        .navigationBarTitleDisplayMode(.inline)
        .alert("Biometric Not Enrolled", isPresented: $showingEnrollmentAlert) {
            Button("Enroll Now") {
                // In a real app, would guide to settings
                diagnostics.logEvent(DiagnosticEvent(
                    type: .dialogDismiss,
                    identifier: "enrollment_alert",
                    details: "User tapped Enroll Now"
                ))
            }
            .accessibilityIdentifier("enroll_now_button")
            
            Button("Cancel", role: .cancel) {
                diagnostics.logEvent(DiagnosticEvent(
                    type: .dialogDismiss,
                    identifier: "enrollment_alert",
                    details: "User cancelled"
                ))
            }
            .accessibilityIdentifier("enrollment_cancel_button")
        } message: {
            Text("Face ID is not set up. Please enroll in Settings to use biometric authentication.")
                .accessibilityIdentifier("enrollment_alert_message")
        }
    }
    
    private var biometricTypeString: String {
        switch biometricType {
        case .faceID:
            return "Face ID"
        case .touchID:
            return "Touch ID"
        case .none:
            return "None"
        case .opticID:
            return "Optic ID"
        @unknown default:
            return "Unknown"
        }
    }
    
    private var isBiometricAvailable: Bool {
        let context = LAContext()
        var error: NSError?
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }
    
    private var isBiometricEnrolled: Bool {
        let context = LAContext()
        var error: NSError?
        let canEvaluate = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
        
        if !canEvaluate, let error = error {
            return error.code != LAError.biometryNotEnrolled.rawValue
        }
        return canEvaluate
    }
    
    // Test Methods
    private func testBiometricSuccess() {
        let result = BiometricTestResult(
            scenario: "Success Authentication",
            success: true,
            message: "User successfully authenticated with \(biometricTypeString)"
        )
        
        performBiometricTest { context in
            // Simulate success - in real test, would trigger Face ID
            diagnostics.logEvent(DiagnosticEvent(
                type: .biometric,
                details: "Success test - waiting for biometric"
            ))
            
            self.testResults.insert(result, at: 0)
        }
    }
    
    private func testBiometricFailure() {
        let result = BiometricTestResult(
            scenario: "Failed Authentication",
            success: false,
            message: "Authentication failed - biometric not recognized"
        )
        
        performBiometricTest { context in
            diagnostics.logEvent(DiagnosticEvent(
                type: .biometric,
                details: "Failure test - simulating non-matching biometric"
            ))
            
            self.testResults.insert(result, at: 0)
        }
    }
    
    private func testBiometricCancel() {
        let result = BiometricTestResult(
            scenario: "User Cancelled",
            success: false,
            message: "User cancelled biometric prompt"
        )
        
        performBiometricTest { context in
            diagnostics.logEvent(DiagnosticEvent(
                type: .biometric,
                details: "Cancel test - user should tap Cancel"
            ))
            
            self.testResults.insert(result, at: 0)
        }
    }
    
    private func testBiometricLockout() {
        let result = BiometricTestResult(
            scenario: "Biometric Lockout",
            success: false,
            message: "Biometric locked due to too many failed attempts"
        )
        
        diagnostics.logEvent(DiagnosticEvent(
            type: .biometric,
            details: "Lockout test - simulating lockout state"
        ))
        
        testResults.insert(result, at: 0)
    }
    
    private func testBiometricNotEnrolled() {
        if !isBiometricEnrolled {
            showingEnrollmentAlert = true
            diagnostics.logEvent(DiagnosticEvent(
                type: .dialogShow,
                identifier: "enrollment_alert",
                details: "Showing enrollment dialog"
            ))
        } else {
            let result = BiometricTestResult(
                scenario: "Not Enrolled Test",
                success: false,
                message: "Cannot test - biometric is already enrolled"
            )
            testResults.insert(result, at: 0)
        }
    }
    
    private func testPasscodeFallback() {
        let result = BiometricTestResult(
            scenario: "Passcode Fallback",
            success: true,
            message: "User entered passcode after biometric failure"
        )
        
        performBiometricTest(policy: .deviceOwnerAuthentication) { context in
            diagnostics.logEvent(DiagnosticEvent(
                type: .biometric,
                details: "Passcode fallback test - fail biometric then enter passcode"
            ))
            
            self.testResults.insert(result, at: 0)
        }
    }
    
    private func performBiometricTest(
        policy: LAPolicy = .deviceOwnerAuthenticationWithBiometrics,
        completion: @escaping (LAContext) -> Void
    ) {
        let context = LAContext()
        let reason = "Test biometric authentication for Arkavo Reference"
        
        diagnostics.logEvent(DiagnosticEvent(
            type: .dialogShow,
            identifier: "biometric_prompt",
            details: "Showing biometric authentication prompt"
        ))
        
        context.evaluatePolicy(policy, localizedReason: reason) { success, error in
            DispatchQueue.main.async {
                diagnostics.logEvent(DiagnosticEvent(
                    type: .dialogDismiss,
                    identifier: "biometric_prompt",
                    details: "Biometric prompt dismissed - success: \(success)"
                ))
                
                completion(context)
            }
        }
    }
    
    private func runAllTests() {
        isRunningTest = true
        testResults.removeAll()
        
        Task {
            // Run each test with delay
            let tests = [
                testBiometricSuccess,
                testBiometricFailure,
                testBiometricCancel,
                testBiometricLockout,
                testBiometricNotEnrolled,
                testPasscodeFallback
            ]
            
            for (index, test) in tests.enumerated() {
                try? await Task.sleep(nanoseconds: 2_000_000_000) // 2 seconds
                test()
                
                diagnostics.logEvent(DiagnosticEvent(
                    type: .stateChange,
                    details: "Completed test \(index + 1) of \(tests.count)"
                ))
            }
            
            try? await Task.sleep(nanoseconds: 1_000_000_000)
            isRunningTest = false
        }
    }
}

struct BiometricTestResult: Identifiable {
    let id = UUID()
    let scenario: String
    let success: Bool
    let message: String
    let timestamp = Date()
}

#Preview {
    NavigationView {
        BiometricTestView()
    }
}
