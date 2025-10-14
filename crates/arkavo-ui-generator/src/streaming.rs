use anyhow::Result;
use arkavo_router::Router;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub chunk_type: ChunkType,
    pub content: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChunkType {
    Html,
    Css,
    JavaScript,
}

impl ChunkType {
    #[allow(dead_code)]
    fn as_str(&self) -> &str {
        match self {
            Self::Html => "html",
            Self::Css => "css",
            Self::JavaScript => "js",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedComponent {
    pub html: String,
    pub css: String,
    pub javascript: String,
}

pub struct StreamingGenerator {
    router: Option<Arc<Router>>,
}

impl StreamingGenerator {
    pub fn new(router: Arc<Router>) -> Result<Self> {
        Ok(Self {
            router: Some(router),
        })
    }

    pub fn new_without_router() -> Result<Self> {
        Ok(Self { router: None })
    }

    pub async fn generate_part(
        &self,
        part_name: &str,
        part_description: &str,
        overall_prompt: &str,
    ) -> Result<mpsc::Receiver<StreamChunk>> {
        let (tx, rx) = mpsc::channel(100);

        let prompt = self.build_component_prompt(part_name, part_description, overall_prompt);

        let router = self.router.clone();

        tokio::spawn(async move {
            // Try Gemini first (if available via Router), fall back to local model
            let use_gemini = router
                .as_ref()
                .map(|r| r.is_gemini_available())
                .unwrap_or(false);

            if use_gemini {
                use arkavo_gemini::RestClient;

                let api_key = std::env::var("GEMINI_API_KEY").unwrap();
                let model =
                    std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-pro".to_string());
                let client = RestClient::new(api_key, model);

                // Define JSON schema for structured output
                let schema = json!({
                    "type": "OBJECT",
                    "properties": {
                        "html": {"type": "STRING"},
                        "css": {"type": "STRING"},
                        "js": {"type": "STRING"}
                    },
                    "required": ["html", "css", "js"]
                });

                // Use streaming API with JSON mode for real-time incremental generation
                match client.stream_generate_content_json(&prompt, schema).await {
                    Ok(mut stream) => {
                        let mut json_buffer = String::new();
                        let mut last_sent_html = String::new();
                        let mut last_sent_css = String::new();
                        let mut last_sent_js = String::new();

                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(chunk) => {
                                    if let Some(text) = chunk.text {
                                        json_buffer.push_str(&text);
                                    }

                                    // Try to parse accumulated JSON incrementally
                                    if let Ok(obj) =
                                        serde_json::from_str::<serde_json::Value>(&json_buffer)
                                    {
                                        let html = obj["html"].as_str().unwrap_or_default();
                                        let css = obj["css"].as_str().unwrap_or_default();
                                        let js = obj["js"].as_str().unwrap_or_default();

                                        // Send only diffs to avoid UI thrashing
                                        if html != last_sent_html {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::Html,
                                                    content: html.to_string(),
                                                    done: false,
                                                })
                                                .await;
                                            last_sent_html = html.to_string();
                                        }

                                        if css != last_sent_css {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::Css,
                                                    content: css.to_string(),
                                                    done: false,
                                                })
                                                .await;
                                            last_sent_css = css.to_string();
                                        }

                                        if js != last_sent_js {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::JavaScript,
                                                    content: js.to_string(),
                                                    done: false,
                                                })
                                                .await;
                                            last_sent_js = js.to_string();
                                        }
                                    }

                                    // Check if stream is done
                                    if chunk.done {
                                        // Final parse attempt
                                        if let Ok(obj) =
                                            serde_json::from_str::<serde_json::Value>(&json_buffer)
                                        {
                                            let html = obj["html"].as_str().unwrap_or_default();
                                            let css = obj["css"].as_str().unwrap_or_default();
                                            let js = obj["js"].as_str().unwrap_or_default();

                                            // Send final diffs if any
                                            if html != last_sent_html && !html.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::Html,
                                                        content: html.to_string(),
                                                        done: false,
                                                    })
                                                    .await;
                                            }

                                            if css != last_sent_css && !css.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::Css,
                                                        content: css.to_string(),
                                                        done: false,
                                                    })
                                                    .await;
                                            }

                                            if js != last_sent_js && !js.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::JavaScript,
                                                        content: js.to_string(),
                                                        done: false,
                                                    })
                                                    .await;
                                            }
                                        }

                                        break;
                                    }
                                }
                                Err(e) => {
                                    // Track error in health system
                                    let health_reporter =
                                        crate::health::UiGeneratorHealthReporter::global();
                                    health_reporter
                                        .record_error(
                                            "Gemini Streaming Error".to_string(),
                                            format!("{}", e),
                                        )
                                        .await;

                                    // If no accumulated text, fall back to local model
                                    if json_buffer.is_empty()
                                        && let Some(router_instance) = &router
                                    {
                                        let classifier = router_instance.get_local_provider();
                                        let messages = vec![arkavo_llm::Message {
                                            role: arkavo_llm::Role::User,
                                            content: prompt.clone(),
                                            images: None,
                                        }];

                                        if let Ok(response) = classifier.complete(messages).await {
                                            json_buffer = response;
                                        }
                                    }

                                    // Parse and send whatever we have (salvaged or fallback)
                                    if !json_buffer.is_empty() {
                                        // Try JSON first
                                        if let Ok(obj) =
                                            serde_json::from_str::<serde_json::Value>(&json_buffer)
                                        {
                                            let html = obj["html"].as_str().unwrap_or_default();
                                            let css = obj["css"].as_str().unwrap_or_default();
                                            let js = obj["js"].as_str().unwrap_or_default();

                                            if !html.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::Html,
                                                        content: html.to_string(),
                                                        done: false,
                                                    })
                                                    .await;
                                            }

                                            if !css.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::Css,
                                                        content: css.to_string(),
                                                        done: false,
                                                    })
                                                    .await;
                                            }

                                            if !js.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::JavaScript,
                                                        content: js.to_string(),
                                                        done: false,
                                                    })
                                                    .await;
                                            }
                                        } else if let Some((html, css, js)) =
                                            Self::try_parse_markdown_fallback(&json_buffer)
                                        {
                                            if !html.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::Html,
                                                        content: html,
                                                        done: false,
                                                    })
                                                    .await;
                                            }
                                            if !css.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::Css,
                                                        content: css,
                                                        done: false,
                                                    })
                                                    .await;
                                            }
                                            if !js.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk {
                                                        chunk_type: ChunkType::JavaScript,
                                                        content: js,
                                                        done: false,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Track error in health system
                        let health_reporter = crate::health::UiGeneratorHealthReporter::global();
                        health_reporter
                            .record_error("Gemini API Error".to_string(), format!("{}", e))
                            .await;

                        // Fall back to local model
                        if let Some(router_instance) = &router {
                            let classifier = router_instance.get_local_provider();
                            let messages = vec![arkavo_llm::Message {
                                role: arkavo_llm::Role::User,
                                content: prompt.clone(),
                                images: None,
                            }];

                            match classifier.complete(messages).await {
                                Ok(response) => {
                                    // Try JSON first, fallback to markdown
                                    if let Ok(obj) =
                                        serde_json::from_str::<serde_json::Value>(&response)
                                    {
                                        let html = obj["html"].as_str().unwrap_or_default();
                                        let css = obj["css"].as_str().unwrap_or_default();
                                        let js = obj["js"].as_str().unwrap_or_default();

                                        if !html.is_empty() {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::Html,
                                                    content: html.to_string(),
                                                    done: false,
                                                })
                                                .await;
                                        }

                                        if !css.is_empty() {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::Css,
                                                    content: css.to_string(),
                                                    done: false,
                                                })
                                                .await;
                                        }

                                        if !js.is_empty() {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::JavaScript,
                                                    content: js.to_string(),
                                                    done: false,
                                                })
                                                .await;
                                        }
                                    } else if let Some((html, css, js)) =
                                        Self::try_parse_markdown_fallback(&response)
                                    {
                                        if !html.is_empty() {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::Html,
                                                    content: html,
                                                    done: false,
                                                })
                                                .await;
                                        }
                                        if !css.is_empty() {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::Css,
                                                    content: css,
                                                    done: false,
                                                })
                                                .await;
                                        }
                                        if !js.is_empty() {
                                            let _ = tx
                                                .send(StreamChunk {
                                                    chunk_type: ChunkType::JavaScript,
                                                    content: js,
                                                    done: false,
                                                })
                                                .await;
                                        }
                                    }
                                }
                                Err(_e) => {
                                    // Local model fallback failed
                                }
                            }
                        }
                    }
                }
            } else if let Some(router_instance) = router {
                // Use local model via Router

                let classifier = router_instance.get_local_provider();
                let messages = vec![arkavo_llm::Message {
                    role: arkavo_llm::Role::User,
                    content: prompt.clone(),
                    images: None,
                }];

                match classifier.complete(messages).await {
                    Ok(response) => {
                        // Try JSON first, fallback to markdown
                        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&response) {
                            let html = obj["html"].as_str().unwrap_or_default();
                            let css = obj["css"].as_str().unwrap_or_default();
                            let js = obj["js"].as_str().unwrap_or_default();

                            // Send complete chunks (local model doesn't stream)
                            if !html.is_empty() {
                                let _ = tx
                                    .send(StreamChunk {
                                        chunk_type: ChunkType::Html,
                                        content: html.to_string(),
                                        done: false,
                                    })
                                    .await;
                            }

                            if !css.is_empty() {
                                let _ = tx
                                    .send(StreamChunk {
                                        chunk_type: ChunkType::Css,
                                        content: css.to_string(),
                                        done: false,
                                    })
                                    .await;
                            }

                            if !js.is_empty() {
                                let _ = tx
                                    .send(StreamChunk {
                                        chunk_type: ChunkType::JavaScript,
                                        content: js.to_string(),
                                        done: false,
                                    })
                                    .await;
                            }
                        } else if let Some((html, css, js)) =
                            Self::try_parse_markdown_fallback(&response)
                        {
                            if !html.is_empty() {
                                let _ = tx
                                    .send(StreamChunk {
                                        chunk_type: ChunkType::Html,
                                        content: html,
                                        done: false,
                                    })
                                    .await;
                            }
                            if !css.is_empty() {
                                let _ = tx
                                    .send(StreamChunk {
                                        chunk_type: ChunkType::Css,
                                        content: css,
                                        done: false,
                                    })
                                    .await;
                            }
                            if !js.is_empty() {
                                let _ = tx
                                    .send(StreamChunk {
                                        chunk_type: ChunkType::JavaScript,
                                        content: js,
                                        done: false,
                                    })
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        // Track error in health system
                        let health_reporter = crate::health::UiGeneratorHealthReporter::global();
                        health_reporter
                            .record_error("Local Model Error".to_string(), format!("{}", e))
                            .await;
                    }
                }
            }

            // Send final done signal
            let _ = tx
                .send(StreamChunk {
                    chunk_type: ChunkType::Html,
                    content: String::new(),
                    done: true,
                })
                .await;
        });

        Ok(rx)
    }

    fn build_component_prompt(&self, name: &str, description: &str, context: &str) -> String {
        format!(
            r#"Generate a web component for: {name}

Description: {description}
Overall Context: {context}

Requirements:
- Provide complete, production-ready HTML, CSS, and JavaScript
- Use semantic HTML5 with proper structure
- Make it fully accessible (ARIA labels, keyboard navigation, screen reader support)
- Use modern CSS (flexbox/grid, CSS variables for theming)
- Dark theme by default with these colors:
  - Background: #0f172a (slate-900)
  - Secondary: #1e293b (slate-800)
  - Text: #e2e8f0 (slate-200)
  - Accent: #3b82f6 (blue-500)
  - Border: #334155 (slate-700)
- NO placeholders, TODOs, or "Add your X here" comments
- NO external dependencies (vanilla JS only, no frameworks)
- Include realistic sample data that demonstrates functionality
- Responsive design (works on mobile and desktop)
- Smooth animations and transitions (use CSS transitions)
- Interactive elements should have hover/focus states
- If data-driven, show at least 3-5 realistic examples

CSS Best Practices:
- Use CSS custom properties for colors/spacing
- Mobile-first responsive design
- Proper spacing with consistent units (rem/em)
- Smooth transitions (0.2s ease-in-out)

JavaScript Best Practices:
- Use modern ES6+ syntax (const/let, arrow functions, template literals)
- Add event listeners properly (no inline handlers)
- Handle edge cases (empty states, loading states)
- Use meaningful variable names

IMPORTANT: Return ONLY valid JSON with keys "html", "css", and "js". Do NOT use markdown code fences.
Your response must be valid JSON that can be parsed directly.

Example format:
{{"html": "<div class='component'>...</div>", "css": ".component {{ ... }}", "js": "// JavaScript code here"}}

Generate the complete, production-ready component now:"#
        )
    }

    fn try_parse_markdown_fallback(response: &str) -> Option<(String, String, String)> {
        let html_pattern = Regex::new(r"(?s)```html\s*(.*?)```").ok()?;
        let css_pattern = Regex::new(r"(?s)```css\s*(.*?)```").ok()?;
        let js_pattern = Regex::new(r"(?s)```(?:javascript|js)\s*(.*?)```").ok()?;

        let html = html_pattern
            .captures(response)?
            .get(1)?
            .as_str()
            .trim()
            .to_string();

        let css = css_pattern
            .captures(response)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        let js = js_pattern
            .captures(response)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        if html.is_empty() {
            None
        } else {
            Some((html, css, js))
        }
    }
}

#[cfg(test)]
mod tests {
    

    #[test]
    fn test_parse_json_component() {
        let response = r#"{"html": "<div class=\"test\">Hello</div>", "css": ".test { color: red; }", "js": "console.log('test');"}"#;

        let obj: serde_json::Value = serde_json::from_str(response).unwrap();
        let html = obj["html"].as_str().unwrap();
        let css = obj["css"].as_str().unwrap();
        let js = obj["js"].as_str().unwrap();

        assert!(html.contains("test"));
        assert!(css.contains("color"));
        assert!(js.contains("console"));
    }
}
