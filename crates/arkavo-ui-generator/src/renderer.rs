use anyhow::{Result, anyhow};
use regex::Regex;

pub struct ParsedCode {
    pub html: String,
    pub css: String,
    pub javascript: String,
}

pub struct CodeRenderer {
    html_pattern: Regex,
    css_pattern: Regex,
    js_pattern: Regex,
}

impl CodeRenderer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            html_pattern: Regex::new(r"```html\n([\s\S]*?)```")?,
            css_pattern: Regex::new(r"```css\n([\s\S]*?)```")?,
            js_pattern: Regex::new(r"```javascript\n([\s\S]*?)```")?,
        })
    }

    pub fn parse_and_render(&self, llm_output: String) -> Result<ParsedCode> {
        let html = self.extract_code_block(&llm_output, &self.html_pattern)?;
        let css = self
            .extract_code_block(&llm_output, &self.css_pattern)
            .unwrap_or_default();
        let javascript = self
            .extract_code_block(&llm_output, &self.js_pattern)
            .unwrap_or_default();

        if html.trim().is_empty() {
            return Err(anyhow!("No HTML content generated"));
        }

        Ok(ParsedCode {
            html,
            css,
            javascript,
        })
    }

    fn extract_code_block(&self, text: &str, pattern: &Regex) -> Result<String> {
        pattern
            .captures(text)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
            .ok_or_else(|| anyhow!("Code block not found"))
    }

    pub fn wrap_component(&self, code: &ParsedCode) -> String {
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <style>
        :root {{
            --bg-primary: #0a0a0a;
            --bg-secondary: #1a1a1a;
            --text-primary: #e0e0e0;
            --text-secondary: #a0a0a0;
            --border-color: #333333;
            --accent-color: #3498db;
        }}

        body {{
            margin: 0;
            padding: 20px;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: var(--bg-primary);
            color: var(--text-primary);
        }}

        {}
    </style>
</head>
<body>
    {}
    <script>
        {}
    </script>
</body>
</html>"#,
            code.css, code.html, code.javascript
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_parsing() {
        let renderer = CodeRenderer::new().unwrap();
        let llm_output = r#"
Here's your component:

```html
<div class="chart-container">
    <canvas id="myChart"></canvas>
</div>
```

```css
.chart-container {
    width: 100%;
    max-width: 600px;
}
```

```javascript
console.log('Chart initialized');
```
"#;

        let result = renderer.parse_and_render(llm_output.to_string());
        assert!(result.is_ok());
        let code = result.unwrap();
        assert!(code.html.contains("chart-container"));
        assert!(code.css.contains("max-width"));
        assert!(code.javascript.contains("Chart initialized"));
    }

    #[test]
    fn test_component_wrapping() {
        let renderer = CodeRenderer::new().unwrap();
        let code = ParsedCode {
            html: "<div>Test</div>".to_string(),
            css: ".test { color: red; }".to_string(),
            javascript: "console.log('hi');".to_string(),
        };

        let wrapped = renderer.wrap_component(&code);
        assert!(wrapped.contains("<!DOCTYPE html>"));
        assert!(wrapped.contains("Test"));
        assert!(wrapped.contains("color: red"));
    }
}
