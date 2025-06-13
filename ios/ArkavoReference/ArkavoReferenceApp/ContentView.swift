import SwiftUI
import LocalAuthentication

struct ContentView: View {
    @StateObject private var authManager = AuthenticationManager()
    @State private var showingLoginView = true
    @State private var showingTestComponents = false
    @EnvironmentObject var navigationManager: NavigationManager
    
    var body: some View {
        NavigationView {
            // Always show calibration view for automation purposes
            CalibrationView()
        }
        .sheet(isPresented: $showingTestComponents) {
            TestComponentsView()
        }
    }
}

struct MainMenuView: View {
    @ObservedObject var authManager: AuthenticationManager
    @Binding var showingTestComponents: Bool
    
    var body: some View {
        VStack(spacing: 20) {
            Text("Welcome to Arkavo Reference")
                .font(.largeTitle)
                .accessibilityIdentifier("welcome_title")
            
            Text("Logged in as: \(authManager.currentUser)")
                .accessibilityIdentifier("logged_in_user")
            
            VStack(spacing: 16) {
                Button("UI Test Components") {
                    showingTestComponents = true
                }
                .buttonStyle(PrimaryButtonStyle())
                .accessibilityIdentifier("test_components_button")
                
                NavigationLink(destination: CalibrationView()) {
                    Text("Calibration Mode")
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(Color.orange)
                        .foregroundColor(.white)
                        .cornerRadius(10)
                }
                .accessibilityIdentifier("calibration_button")
                
                Button("Coordinate Visualizer") {
                    // Navigate to coordinate visualizer
                }
                .buttonStyle(PrimaryButtonStyle())
                .accessibilityIdentifier("coordinate_visualizer_button")
                
                Button("Diagnostic Panel") {
                    // Navigate to diagnostic panel
                }
                .buttonStyle(PrimaryButtonStyle())
                .accessibilityIdentifier("diagnostic_panel_button")
                
                Button("Sign Out") {
                    authManager.signOut()
                }
                .buttonStyle(SecondaryButtonStyle())
                .accessibilityIdentifier("sign_out_button")
            }
            .padding()
        }
        .navigationTitle("Main Menu")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct LoginView: View {
    @ObservedObject var authManager: AuthenticationManager
    @Binding var showingLoginView: Bool
    
    @State private var username = ""
    @State private var password = ""
    @State private var showingBiometricOption = false
    @State private var loginAttempts = 0
    @State private var isLocked = false
    @State private var showingError = false
    @State private var errorMessage = ""
    
    var body: some View {
        VStack(spacing: 20) {
            Text("Arkavo Reference App")
                .font(.largeTitle)
                .accessibilityIdentifier("app_title")
            
            Image(systemName: "lock.shield")
                .font(.system(size: 80))
                .foregroundColor(.blue)
                .accessibilityIdentifier("app_logo")
            
            if isLocked {
                Text("Account locked due to multiple failed attempts")
                    .foregroundColor(.red)
                    .accessibilityIdentifier("account_locked_message")
            } else {
                VStack(spacing: 16) {
                    TextField("Username", text: $username)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .autocapitalization(.none)
                        .accessibilityIdentifier("username_field")
                    
                    SecureField("Password", text: $password)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .accessibilityIdentifier("password_field")
                    
                    Button("Sign In") {
                        attemptLogin()
                    }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(username.isEmpty || password.isEmpty)
                    .accessibilityIdentifier("sign_in_button")
                    
                    if authManager.isBiometricAvailable {
                        Button("Use Face ID") {
                            authenticateWithBiometric()
                        }
                        .buttonStyle(SecondaryButtonStyle())
                        .accessibilityIdentifier("face_id_button")
                    }
                    
                    Button("Create Account") {
                        showingLoginView = false
                    }
                    .foregroundColor(.blue)
                    .accessibilityIdentifier("create_account_button")
                }
                .padding()
            }
        }
        .padding()
        .alert("Login Error", isPresented: $showingError) {
            Button("OK") { }
        } message: {
            Text(errorMessage)
        }
        .navigationTitle("Sign In")
        .navigationBarTitleDisplayMode(.inline)
    }
    
    private func attemptLogin() {
        loginAttempts += 1
        
        if loginAttempts >= 5 {
            isLocked = true
            errorMessage = "Account locked after 5 failed attempts"
            showingError = true
            return
        }
        
        if authManager.signIn(username: username, password: password) {
            // Success - handled by authManager
        } else {
            errorMessage = "Invalid credentials. Attempt \(loginAttempts) of 5"
            showingError = true
        }
    }
    
    private func authenticateWithBiometric() {
        authManager.authenticateWithBiometric { success, error in
            if !success {
                errorMessage = error ?? "Biometric authentication failed"
                showingError = true
            }
        }
    }
}

struct RegistrationView: View {
    @ObservedObject var authManager: AuthenticationManager
    @Binding var showingLoginView: Bool
    
    @State private var username = ""
    @State private var password = ""
    @State private var confirmPassword = ""
    @State private var enableBiometric = false
    @State private var acceptTerms = false
    @State private var showingError = false
    @State private var errorMessage = ""
    
    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                Text("Create Account")
                    .font(.largeTitle)
                    .accessibilityIdentifier("registration_title")
                
                VStack(spacing: 16) {
                    TextField("Username", text: $username)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .autocapitalization(.none)
                        .accessibilityIdentifier("reg_username_field")
                    
                    SecureField("Password", text: $password)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .accessibilityIdentifier("reg_password_field")
                    
                    SecureField("Confirm Password", text: $confirmPassword)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .accessibilityIdentifier("reg_confirm_password_field")
                    
                    Toggle("Enable Face ID", isOn: $enableBiometric)
                        .accessibilityIdentifier("enable_biometric_toggle")
                    
                    HStack {
                        Button(action: { acceptTerms.toggle() }) {
                            Image(systemName: acceptTerms ? "checkmark.square" : "square")
                                .foregroundColor(.blue)
                                .accessibilityIdentifier("terms_checkbox")
                        }
                        
                        Text("I accept the terms and conditions")
                            .accessibilityIdentifier("terms_label")
                        
                        Spacer()
                    }
                    
                    Button("Create Account") {
                        createAccount()
                    }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(!canCreateAccount)
                    .accessibilityIdentifier("create_account_submit_button")
                    
                    Button("Back to Sign In") {
                        showingLoginView = true
                    }
                    .foregroundColor(.blue)
                    .accessibilityIdentifier("back_to_login_button")
                }
                .padding()
            }
        }
        .alert("Registration Error", isPresented: $showingError) {
            Button("OK") { }
        } message: {
            Text(errorMessage)
        }
        .navigationTitle("Register")
        .navigationBarTitleDisplayMode(.inline)
    }
    
    private var canCreateAccount: Bool {
        !username.isEmpty &&
        !password.isEmpty &&
        password == confirmPassword &&
        acceptTerms
    }
    
    private func createAccount() {
        guard canCreateAccount else { return }
        
        if password != confirmPassword {
            errorMessage = "Passwords do not match"
            showingError = true
            return
        }
        
        if authManager.register(username: username, password: password, enableBiometric: enableBiometric) {
            showingLoginView = true
        } else {
            errorMessage = "Failed to create account"
            showingError = true
        }
    }
}

// Button Styles
struct PrimaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(maxWidth: .infinity)
            .padding()
            .background(Color.blue)
            .foregroundColor(.white)
            .cornerRadius(10)
            .scaleEffect(configuration.isPressed ? 0.95 : 1.0)
    }
}

struct SecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(maxWidth: .infinity)
            .padding()
            .background(Color.gray.opacity(0.2))
            .foregroundColor(.blue)
            .cornerRadius(10)
            .scaleEffect(configuration.isPressed ? 0.95 : 1.0)
    }
}

// Authentication Manager
class AuthenticationManager: ObservableObject {
    @Published var isAuthenticated = false
    @Published var currentUser = ""
    
    private let context = LAContext()
    
    var isBiometricAvailable: Bool {
        var error: NSError?
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }
    
    func signIn(username: String, password: String) -> Bool {
        // Simple demo authentication
        if username == "demo" && password == "password" {
            isAuthenticated = true
            currentUser = username
            return true
        }
        return false
    }
    
    func register(username: String, password: String, enableBiometric: Bool) -> Bool {
        // Simple demo registration
        // In real app, would save to keychain/database
        return true
    }
    
    func authenticateWithBiometric(completion: @escaping (Bool, String?) -> Void) {
        let reason = "Sign in to Arkavo Reference"
        
        context.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, localizedReason: reason) { success, error in
            DispatchQueue.main.async {
                if success {
                    self.isAuthenticated = true
                    self.currentUser = "biometric_user"
                    completion(true, nil)
                } else {
                    completion(false, error?.localizedDescription)
                }
            }
        }
    }
    
    func signOut() {
        isAuthenticated = false
        currentUser = ""
    }
}

#Preview {
    ContentView()
}