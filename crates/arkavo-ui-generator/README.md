# Arkavo UI Generator

AI-driven UI code generation with vision capabilities for Arkavo Edge.

## Features

- **Dynamic UI Generation**: Generate HTML/CSS/JavaScript from natural language prompts
- **Vision-Powered Verification**: Screenshot capture and LLM-based UI validation
- **Router Integration**: Intelligent model selection for cost-aware generation
- **Streaming Support**: Real-time UI generation with progress tracking

## Vision Capabilities

The UI generator includes vision support for screenshot capture and automated UI verification using multimodal language models.

### Supported Models

✅ **Qwen2.5-VL** (3B/7B/32B/72B) - Officially supported by llama.cpp via mtmd API
❌ **Qwen3-VL** - Not yet supported (see [llama.cpp issue #16207](https://github.com/ggml-org/llama.cpp/issues/16207))

This implementation uses **Qwen2.5-VL**, which is production-ready and officially supported. Qwen3-VL support can be added when upstream llama.cpp support becomes available.

### Architecture

Vision integration uses llama.cpp's multimodal API (mtmd) with the following components:

- **Screenshot Capture**: Platform-specific capture (screencapture on macOS, ImageMagick on Linux)
- **Image Preprocessing**: Automatic resizing to 448x448 RGB for CLIP vision encoder
- **Multimodal LLM**: Qwen2.5-VL (3B/7B/32B/72B) via llama.cpp with mmproj support
- **UI Verification**: Automated pass/fail validation against requirements

### Model Setup

Models are automatically downloaded from HuggingFace on first use. No manual download required!

```rust
use arkavo_ui_generator::vision::{Qwen25VLModelLoader, ModelSize};

// Automatically downloads and caches the model
let loader = Qwen25VLModelLoader::new()?;
let paths = loader.ensure_model(ModelSize::Medium7B).await?;
```

Available model sizes:
- `ModelSize::Small3B` - 3B parameters (faster, less accurate)
- `ModelSize::Medium7B` - 7B parameters (balanced, recommended)
- `ModelSize::Large32B` - 32B parameters (slower, most accurate)

Models are cached in `~/.arkavo/models/qwen25vl/` for reuse.

#### Manual Download (Optional)

If you prefer to download manually:

```bash
# Download Qwen2.5-VL-7B GGUF model (Q8_0 quantization)
wget https://huggingface.co/Mungert/Qwen2.5-VL-7B-Instruct-GGUF/resolve/main/Qwen2.5-VL-7B-Instruct-Q8_0.gguf

# Download mmproj projector file (vision encoder)
wget https://huggingface.co/Mungert/Qwen2.5-VL-7B-Instruct-GGUF/resolve/main/Qwen2.5-VL-7B-Instruct-mmproj-f16.gguf
```

Place both files in `~/.arkavo/models/qwen25vl/`.

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
    "qwen25vl".to_string(),
    "path/to/Qwen2.5-VL-7B-Instruct-Q8_0.gguf".to_string(),
    "path/to/Qwen2.5-VL-7B-Instruct-mmproj-f16.gguf".to_string(),
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

## MCP Tool Integration (TODO)

### Current Status: Not Integrated

The UI generator's `UiPlanner` and `StreamingGenerator` currently **do not use Router's MCP tool integration**.

**Current Flow** (`planner.rs` lines 41-62):
```rust
// Current: Direct provider call without tools
async fn try_llm_plan(&self, prompt: &str) -> Result<String> {
    let provider = self.router.get_planning_provider()
        .ok_or_else(|| Error::Config("Gemini not available".to_string()))?;

    let response = provider.complete(messages).await?;  // ❌ No tools!
    Ok(response)
}
```

**Issues:**
- Cannot access MCP tools (filesystem, GitHub, browser, etc.)
- No quality validation (hallucinated tools, refusals)
- No automatic model escalation on poor responses

### How to Add Tool Support

**Step 1: Add `plan_with_tools()` Method** (`planner.rs`):
```rust
use arkavo_mcp_tools::ToolRegistry;
use arkavo_llm::{Message, Role, ProviderResponse};

pub async fn plan_with_tools(
    &self,
    prompt: &str,
    tool_registry: Option<&ToolRegistry>,
) -> Result<String> {
    let messages = vec![Message {
        role: Role::User,
        content: self.build_planning_prompt(prompt),
        images: None,
    }];

    // Use Router's quality gate for validation + escalation
    let response: ProviderResponse = self.router.route_with_quality_gate(
        prompt,
        messages,
        tool_registry,
        3,  // Max retries with model escalation
    ).await.map_err(|e| Error::Router(e))?;

    // Handle tool calls if LLM requested them
    if !response.tool_calls.is_empty() && tool_registry.is_some() {
        let registry = tool_registry.unwrap();
        for tool_call in &response.tool_calls {
            if let Some(tool) = registry.get(&tool_call.tool_name) {
                let result = tool.execute(tool_call.arguments.clone()).await
                    .map_err(|e| Error::ToolExecution(e.to_string()))?;
                // Feed result back to LLM for plan refinement
            }
        }
    }

    Ok(response.content)
}
```

**Step 2: Add `generate_part_with_tools()` Method** (`streaming.rs`):
```rust
pub async fn generate_part_with_tools(
    &self,
    part_spec: &str,
    tool_registry: Option<&ToolRegistry>,
) -> Result<BoxStream<'static, Result<String>>> {
    let messages = vec![Message {
        role: Role::User,
        content: format!("{}\n\nGenerate: {}", SYSTEM_PROMPT, part_spec),
        images: None,
    }];

    // Use route_with_quality_gate for validation
    let response = self.router.route_with_quality_gate(
        part_spec,
        messages,
        tool_registry,
        2,  // Fewer retries for streaming (faster)
    ).await.map_err(|e| Error::Router(e))?;

    // Stream the response content
    Ok(Box::pin(futures::stream::once(async move {
        Ok(response.content)
    })))
}
```

**Step 3: Update Gateway Integration** (caller code):
```rust
// In arkavo-agui/src/gateway.rs
let tool_registry = ToolRegistry::new();

// Use new methods
let plan = planner.plan_with_tools(&prompt, Some(&tool_registry)).await?;
let stream = generator.generate_part_with_tools(&part, Some(&tool_registry)).await?;
```

### Benefits After Integration

✅ **Access to MCP tools** - Can call filesystem, GitHub, browser tools during planning/generation
✅ **Quality validation** - Fast validation (<1ms) catches hallucinated tools instantly
✅ **LLM judge** - Gemma 4B evaluates responses for refusals, off-topic content (~500ms)
✅ **Auto-escalation** - Automatically retries with better models (270M→4B→12B→Flash→Pro)
✅ **Better UX** - Tool execution feedback improves plan quality
✅ **Cost optimization** - Tries local models first, escalates only when needed

### Example: UI Generation with Tools

```bash
# After integration, this will work:
arkavo ui --prompt "Show me the repository structure and generate a file browser UI"

# The planner will:
# 1. Call filesystem__list_directory tool to get actual file list
# 2. Generate plan based on real data (not hallucinated files)
# 3. If response quality is poor, automatically retry with better model
# 4. Stream validated HTML/CSS/JS to browser
```

### Integration Points

| Component | File | Lines | Change Required |
|-----------|------|-------|-----------------|
| **Planner** | `planner.rs` | 41-62 | Add `plan_with_tools()` method |
| **Streaming** | `streaming.rs` | 68-180 | Add `generate_part_with_tools()` method |
| **Gateway** | `../arkavo-agui/src/gateway.rs` | 962-967 | Call new methods with ToolRegistry |

### See Also

- `crates/arkavo-router/README.md` - Router quality gate documentation and integration guide
- `crates/arkavo-agui/README.md` - Gateway integration instructions

## Resources

- [Qwen3-VL Model Card](https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct)
- [llama.cpp Multimodal API](https://github.com/ggerganov/llama.cpp/tree/master/tools/mtmd)
- [CLIP Vision Encoder](https://github.com/openai/CLIP)
- [Integration Tests](tests/README.md)
- [arkavo-router Quality Gate](../arkavo-router/README.md#quality-gate-for-mcp-tool-calling)
