use anyhow::Result;
use arkavo_router::Router;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
// StreamExt trait provides .next() method for stream iteration
#[allow(unused_imports)]
use tokio_stream::StreamExt;

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
    html_pattern: Regex,
    css_pattern: Regex,
    js_pattern: Regex,
}

impl StreamingGenerator {
    pub fn new(router: Arc<Router>) -> Result<Self> {
        Ok(Self {
            router: Some(router),
            html_pattern: Regex::new(r"(?s)```html\s*(.*?)```")?,
            css_pattern: Regex::new(r"(?s)```css\s*(.*?)```")?,
            js_pattern: Regex::new(r"(?s)```(?:javascript|js)\s*(.*?)```")?,
        })
    }

    pub fn new_without_router() -> Result<Self> {
        Ok(Self {
            router: None,
            html_pattern: Regex::new(r"(?s)```html\s*(.*?)```")?,
            css_pattern: Regex::new(r"(?s)```css\s*(.*?)```")?,
            js_pattern: Regex::new(r"(?s)```(?:javascript|js)\s*(.*?)```")?,
        })
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
        let html_pattern = self.html_pattern.clone();
        let css_pattern = self.css_pattern.clone();
        let js_pattern = self.js_pattern.clone();

        tokio::spawn(async move {
            if let Some(router_instance) = router {
                let _decision = router_instance.route(&prompt).await;
            }

            if let Ok(api_key) = std::env::var("GEMINI_API_KEY") {
                use arkavo_gemini::RestClient;

                let model =
                    std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-pro".to_string());
                let client = RestClient::new(api_key, model);

                // Use streaming API for real-time incremental generation
                match client.stream_generate_content(&prompt, None).await {
                    Ok(mut stream) => {
                        let mut accumulated_text = String::new();

                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(chunk) => {
                                    if let Some(text) = chunk.text {
                                        accumulated_text.push_str(&text);
                                    }

                                    // Parse accumulated text to extract HTML/CSS/JS incrementally
                                    let component = Self::parse_component(
                                        &accumulated_text,
                                        &html_pattern,
                                        &css_pattern,
                                        &js_pattern,
                                    );

                                    // Send incremental updates as they become available
                                    if !component.html.is_empty() {
                                        let _ = tx
                                            .send(StreamChunk {
                                                chunk_type: ChunkType::Html,
                                                content: component.html.clone(),
                                                done: false,
                                            })
                                            .await;
                                    }

                                    if !component.css.is_empty() {
                                        let _ = tx
                                            .send(StreamChunk {
                                                chunk_type: ChunkType::Css,
                                                content: component.css.clone(),
                                                done: false,
                                            })
                                            .await;
                                    }

                                    if !component.javascript.is_empty() {
                                        let _ = tx
                                            .send(StreamChunk {
                                                chunk_type: ChunkType::JavaScript,
                                                content: component.javascript.clone(),
                                                done: false,
                                            })
                                            .await;
                                    }

                                    // Check if stream is done
                                    if chunk.done {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Gemini streaming error: {e}");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Gemini API error: {e}");
                    }
                }
            } else {
                eprintln!("GEMINI_API_KEY not set, using fallback");
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

Format your response as:
```html
<div class="component-name">
  <!-- Your complete HTML structure here -->
</div>
```

```css
.component-name {{
  /* Your complete CSS with custom properties */
}}
```

```javascript
// Your complete JavaScript implementation (if needed)
// Include event handlers, data manipulation, etc.
```

Generate the complete, production-ready component now:"#
        )
    }

    fn parse_component(
        response: &str,
        html_pattern: &Regex,
        css_pattern: &Regex,
        js_pattern: &Regex,
    ) -> GeneratedComponent {
        let html = html_pattern
            .captures(response)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        let css = css_pattern
            .captures(response)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        let javascript = js_pattern
            .captures(response)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        GeneratedComponent {
            html,
            css,
            javascript,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_component() {
        let html_pattern = Regex::new(r"(?s)```html\s*(.*?)```").unwrap();
        let css_pattern = Regex::new(r"(?s)```css\s*(.*?)```").unwrap();
        let js_pattern = Regex::new(r"(?s)```(?:javascript|js)\s*(.*?)```").unwrap();

        let response = r#"
        Here's the component:

        ```html
        <div class="test">Hello</div>
        ```

        ```css
        .test { color: red; }
        ```

        ```javascript
        console.log('test');
        ```
        "#;

        let component =
            StreamingGenerator::parse_component(response, &html_pattern, &css_pattern, &js_pattern);

        assert!(component.html.contains("test"));
        assert!(component.css.contains("color"));
        assert!(component.javascript.contains("console"));
    }
}
