use anyhow::Result;
use arkavo_router::Router;
use regex::Regex;
use serde::{Deserialize, Serialize};
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

                match client.generate_content(&prompt, None).await {
                    Ok((Some(response_text), _)) => {
                        let component = Self::parse_component(
                            &response_text,
                            &html_pattern,
                            &css_pattern,
                            &js_pattern,
                        );

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
                    }
                    Ok((None, _)) => {
                        eprintln!("Gemini returned no text");
                    }
                    Err(e) => {
                        eprintln!("Gemini API error: {e}");
                    }
                }
            } else {
                eprintln!("GEMINI_API_KEY not set, using fallback");
            }

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
- Use semantic HTML5
- Make it accessible (ARIA labels, keyboard navigation)
- Use modern CSS (flexbox/grid, no hardcoded sizes)
- Dark theme by default
- No placeholders or TODOs
- No external dependencies (vanilla JS only)
- Include realistic sample data

Format your response as:
```html
<div class="component-name">
  <!-- Your HTML here -->
</div>
```

```css
.component-name {{
  /* Your CSS here */
}}
```

```javascript
// Your JavaScript here (if needed)
```

Generate now:"#
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
