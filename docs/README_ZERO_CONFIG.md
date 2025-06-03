# Arkavo: Zero-Configuration Intelligent Testing

## Just Run It

```bash
# In ANY project directory (iOS, Android, Web, etc.)
arkavo test --explore
```

That's it. No setup. No configuration. No integration steps.

## What Happens

1. **Auto-Detection**: Arkavo automatically detects your project type
2. **Smart Integration**: Injects test harness without modifying your code
3. **AI Analysis**: Understands your domain model and business logic
4. **Bug Discovery**: Finds bugs you didn't know existed
5. **Actionable Reports**: Provides fixes, not just failures

## Examples

### iOS App
```bash
cd MyiOSApp
arkavo test --explore

# Output:
🔍 Auto-detected iOS project
✨ Auto-integrating test harness...
✅ Integrated using: DynamicLibrary
🔧 No manual setup required!

🧠 Analyzing your code...
Found: PaymentViewController with potential race condition
Found: UserManager with missing null checks
Found: NetworkClient with unhandled timeout scenarios

❌ Bug #1: Double charge possible in payment flow
   When: Two payment requests within 10ms
   Fix: Add distributed lock on transaction ID
```

### React Native App
```bash
cd MyReactApp  
arkavo test --explore

# Output:
🔍 Auto-detected React Native project
✨ Auto-integrating test harness...
✅ Integrated using: MetroBundle
🔧 No manual setup required!

🧠 Finding edge cases...
❌ Bug: State update after unmount causes crash
   Fix: Check component mounted state
```

### Any Project
```bash
arkavo test --explore
# It just works™
```

## How It Works

### 1. **Project Detection**
- Scans for `.xcodeproj`, `package.json`, `build.gradle`, etc.
- Identifies project type and structure
- No configuration files needed

### 2. **Runtime Injection**
- **iOS**: Uses DYLD_INSERT_LIBRARIES or runtime swizzling
- **Android**: Uses ADB instrumentation or Frida
- **Web**: Injects via browser DevTools protocol
- **React Native**: Hooks into Metro bundler

### 3. **Intelligent Analysis**
- Analyzes source code with AI
- Understands domain models
- Discovers invariants and properties
- Generates test cases that break assumptions

### 4. **Zero Touch Execution**
- No test files to write
- No setup code needed
- No build modifications
- Just results

## Advanced Usage (Still Zero Config!)

### Find Specific Bugs
```bash
arkavo test --explore --focus "payment"
# Focuses on payment-related code
```

### Verify Properties
```bash
arkavo test --properties
# Discovers and verifies system invariants
```

### Chaos Testing
```bash
arkavo test --chaos
# Injects failures to test resilience
```

### CI Integration
```yaml
# .github/workflows/test.yml
- name: Arkavo Test
  run: arkavo test --explore --ci
```

## Supported Platforms

✅ iOS (Swift/Objective-C)
✅ Android (Kotlin/Java)
✅ React Native
✅ Flutter
✅ Web (React/Vue/Angular)
✅ Node.js
✅ Rust
✅ Go
✅ Python
✅ Java

## No Configuration Required

❌ No config files
❌ No annotations
❌ No test harness setup
❌ No framework integration
❌ No manual steps

Just run `arkavo test --explore` and find bugs.

## The Magic

Arkavo uses:
- **Dynamic instrumentation** to hook into your app
- **AI-powered analysis** to understand your code
- **Intelligent fuzzing** to find edge cases
- **Automatic minimization** to provide clear reproductions

All without touching your codebase.

## Get Started

```bash
# Install
brew install arkavo

# Run in any project
arkavo test --explore

# Watch bugs appear
# 🐛 → 💡 → ✅
```

That's it. No docs to read. No setup guides. It just works.