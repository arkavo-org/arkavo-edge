# Arkavo UI Generator

AI-driven UI code generation with vision capabilities for Arkavo Edge.

## Features

- **Dynamic UI Generation**: Generate HTML/CSS/JavaScript from natural language prompts
- **Vision-Powered Verification**: Screenshot capture and LLM-based UI validation
- **Router Integration**: Intelligent model selection for cost-aware generation
- **Streaming Support**: Real-time UI generation with progress tracking

## Vision Capabilities

The UI generator includes vision support for screenshot capture and automated UI verification using multimodal language models.

### Architecture

Vision integration uses llama.cpp's multimodal API (mtmd) with the following components:

- **Screenshot Capture**: Platform-specific capture (screencapture on macOS, ImageMagick on Linux)
- **Image Preprocessing**: Automatic resizing to 448x448 RGB for CLIP vision encoder
- **Multimodal LLM**: Qwen3-VL-4B-Instruct via llama.cpp with mmproj support
- **UI Verification**: Automated pass/fail validation against requirements

### Model Setup

Download the vision model and mmproj file:

```bash
# Download Qwen3-VL-4B GGUF model (4-bit quantized)
wget https://huggingface.co/unsloth/Qwen3-VL-4B-Instruct-unsloth-bnb-4bit/resolve/main/Qwen3-VL-4B-Instruct-Q4_K_M.gguf

# Download mmproj projector file (vision encoder)
wget https://huggingface.co/unsloth/Qwen3-VL-4B-Instruct-unsloth-bnb-4bit/resolve/main/mmproj-model-f16.gguf
```

Place both files in the same directory (e.g., `~/.arkavo/models/qwen3vl/`).

### Usage

#### Screenshot Capture

Capture a screenshot for vision analysis:

```rust
use arkavo_ui_generator::vision::ScreenshotCapture;

// Interactive window selection (macOS)
let screenshot = ScreenshotCapture::capture_window()?;

// Load from file
let screenshot = ScreenshotCapture::from_file("path/to/image.png")?;

// Get image properties
println!("Size: {}x{}", screenshot.width, screenshot.height);
println!("Format: {:?}", screenshot.format);
```

#### Vision Message Creation

Create a message with image for multimodal LLM:

```rust
use arkavo_ui_generator::vision::ScreenshotCapture;
use arkavo_llm::Message;

let screenshot = ScreenshotCapture::from_file("ui.png")?;
let message = screenshot.create_vision_message("Describe this UI")?;

// message contains both text prompt and base64-encoded image
```

#### UI Verification

Automated UI verification with vision model:

```rust
use arkavo_ui_generator::vision::{ScreenshotCapture, UiVerifier};
use arkavo_llm::LlamaCppProvider;

// Create vision-enabled provider
let provider = LlamaCppProvider::new_with_mmproj(
    "qwen3vl".to_string(),
    "path/to/Qwen3-VL-4B-Instruct-Q4_K_M.gguf".to_string(),
    "path/to/mmproj-model-f16.gguf".to_string(),
)?;

// Capture screenshot
let screenshot = ScreenshotCapture::capture_window()?;

// Verify against requirements
let verifier = UiVerifier::new(Some(Box::new(provider)));
let result = verifier.verify_ui(
    &screenshot,
    "UI should have a dark theme with a prominent search bar"
).await?;

if result.passed {
    println!("✅ {}", result.display_summary());
} else {
    println!("❌ {}", result.display_summary());
}
```

### Platform Support

| Platform | Screenshot Method | Status |
|----------|------------------|--------|
| macOS    | `screencapture` command | ✅ Full support |
| Linux    | ImageMagick `import` | ✅ Full support |
| Windows  | Not implemented | ⏳ Planned |

### Supported Image Formats

- **PNG**: Full support with dimension parsing
- **JPEG**: Full support with SOF marker parsing
- **Base64**: Automatic encoding/decoding for LLM transport

### Vision Model Integration

The vision integration uses arkavo-llama-cpp's multimodal API:

```rust
use arkavo_llm::LlamaCppProvider;

// Standard text-only provider
let text_provider = LlamaCppProvider::new(
    "llama3".to_string(),
    "path/to/model.gguf".to_string(),
)?;

// Vision-enabled provider with mmproj
let vision_provider = LlamaCppProvider::new_with_mmproj(
    "qwen3vl".to_string(),
    "path/to/Qwen3-VL-4B-Instruct-Q4_K_M.gguf".to_string(),
    "path/to/mmproj-model-f16.gguf".to_string(),
)?;

// Provider automatically detects images in messages
let response = vision_provider.generate_streaming(vec![
    message_with_image
]).await?;
```

### Image Processing Pipeline

When a message contains images:

1. **Detection**: Provider checks for `Message.images` field
2. **Decoding**: Base64 image data is decoded to bytes
3. **Preprocessing**: Image is resized to 448x448 RGB (CLIP target size)
4. **Bitmap Creation**: RGB data is wrapped in `MtmdBitmap`
5. **Tokenization**: Text prompt is tokenized with `<__media__>` marker
6. **Encoding**: Image chunks are encoded to embeddings via CLIP
7. **Generation**: Text tokens are generated with visual context

### Testing

Run vision integration tests:

```bash
# Run all vision tests
cargo test -p arkavo-ui-generator --test vision_test

# Run specific test
cargo test -p arkavo-ui-generator --test vision_test test_png_from_file
```

Test coverage includes:
- PNG/JPEG file loading
- Dimension parsing
- Base64 encoding/decoding
- Vision message creation
- Multi-image handling

### Performance Considerations

**Model Loading**:
- First inference: ~2-3 seconds (model + mmproj load)
- Subsequent inferences: <500ms (cached in memory)

**Image Processing**:
- Preprocessing (448x448 resize): ~10ms
- CLIP encoding: ~100-200ms (depends on hardware)
- Text generation: ~20-30 tokens/sec (4-bit Q4_K_M on M3 Mac)

**Memory Usage**:
- Qwen3-VL-4B Q4_K_M: ~2.5GB RAM
- mmproj-model-f16: ~650MB RAM
- Total: ~3.2GB for vision-enabled inference

### Example: Complete Verification Workflow

```rust
use arkavo_ui_generator::{UiGenerator, UiGenerationRequest, UiContext, UiPreferences};
use arkavo_ui_generator::vision::{ScreenshotCapture, UiVerifier};
use arkavo_llm::LlamaCppProvider;

async fn generate_and_verify() -> anyhow::Result<()> {
    // Generate UI
    let generator = UiGenerator::new().await?;
    let request = UiGenerationRequest {
        user_intent: "Build a dark-themed dashboard".to_string(),
        context: UiContext {
            available_agents: vec![],
            active_telemetry: vec![],
            current_page: None,
        },
        preferences: UiPreferences::default(),
    };

    let generated_ui = generator.generate(request).await?;

    // Render in browser (see tests/README.md for browser integration)
    // ... browser rendering code ...

    // Capture screenshot
    let screenshot = ScreenshotCapture::capture_window()?;

    // Verify with vision model
    let provider = LlamaCppProvider::new_with_mmproj(
        "qwen3vl".to_string(),
        "~/.arkavo/models/qwen3vl/Qwen3-VL-4B-Instruct-Q4_K_M.gguf".to_string(),
        "~/.arkavo/models/qwen3vl/mmproj-model-f16.gguf".to_string(),
    )?;

    let verifier = UiVerifier::new(Some(Box::new(provider)));
    let result = verifier.verify_ui(
        &screenshot,
        "Dashboard should have dark theme with metrics cards and charts"
    ).await?;

    println!("{}", result.display_summary());

    Ok(())
}
```

## Architecture

```
┌─────────────────────┐
│   UiGenerator       │
│  - Router           │  ← Model selection
│  - PromptBuilder    │  ← Prompt engineering
│  - CodeRenderer     │  ← HTML/CSS/JS parsing
└──────────┬──────────┘
           │
           ├──────────────────┐
           │                  │
           ▼                  ▼
┌─────────────────────┐  ┌──────────────────┐
│  Vision Module      │  │  Browser Module  │
│  - ScreenCapture    │  │  - CDP Injection │
│  - UiVerifier       │  │  - Screenshots   │
│  - ImageProcessor   │  │  - DOM Mutation  │
└─────────────────────┘  └──────────────────┘
           │
           ▼
┌─────────────────────┐
│  LlamaCppProvider   │
│  - Text Model       │
│  - Vision Model     │
│  - Multimodal API   │
└─────────────────────┘
```

## File Structure

```
arkavo-ui-generator/
├── src/
│   ├── lib.rs              # Main generator logic
│   ├── vision.rs           # Screenshot capture & verification
│   ├── planner.rs          # UI decomposition
│   ├── prompt.rs           # Prompt building
│   ├── renderer.rs         # Code parsing
│   ├── streaming.rs        # Progress tracking
│   ├── templates.rs        # HTML templates
│   └── health.rs           # Health checks
├── tests/
│   ├── vision_test.rs      # Vision integration tests
│   ├── integration_test.rs # E2E browser tests
│   └── README.md           # Test documentation
└── README.md               # This file
```

## Dependencies

Core:
- `arkavo-router`: Intelligent model routing
- `arkavo-llm`: LLM provider abstraction
- `arkavo-llama-cpp`: Native llama.cpp FFI bindings

Vision:
- llama.cpp mtmd API (multimodal support)
- CLIP vision encoder (via mmproj)

Browser:
- `chromiumoxide`: Chrome DevTools Protocol
- `arkavo-browser`: Browser automation utilities

## Contributing

When adding vision features:
- Keep files under 400 lines (project standard)
- Add integration tests for new capabilities
- Document model compatibility and performance
- Use platform-specific compilation flags where needed

## Resources

- [Qwen3-VL Model Card](https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct)
- [llama.cpp Multimodal API](https://github.com/ggerganov/llama.cpp/tree/master/tools/mtmd)
- [CLIP Vision Encoder](https://github.com/openai/CLIP)
- [Integration Tests](tests/README.md)
