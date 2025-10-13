use crate::CodeUpdate;
use anyhow::{Result, anyhow};

pub struct CodeValidator {
    max_html_size: usize,
    max_css_size: usize,
    max_js_size: usize,
    forbidden_patterns: Vec<String>,
}

impl CodeValidator {
    pub fn new() -> Self {
        Self {
            max_html_size: 100_000,
            max_css_size: 50_000,
            max_js_size: 100_000,
            forbidden_patterns: vec![
                "eval(".to_string(),
                "Function(".to_string(),
                "<script src=\"http://".to_string(),
                "document.write(".to_string(),
                "innerHTML =".to_string(),
            ],
        }
    }

    pub fn validate(&self, update: &CodeUpdate) -> Result<()> {
        if let Some(html) = &update.html {
            self.validate_html(html)?;
        }

        if let Some(css) = &update.css {
            self.validate_css(css)?;
        }

        if let Some(js) = &update.javascript {
            self.validate_javascript(js)?;
        }

        Ok(())
    }

    fn validate_html(&self, html: &str) -> Result<()> {
        if html.len() > self.max_html_size {
            return Err(anyhow!(
                "HTML exceeds maximum size: {} > {}",
                html.len(),
                self.max_html_size
            ));
        }

        let balanced = self.check_balanced_tags(html);
        if !balanced {
            return Err(anyhow!("HTML has unbalanced tags"));
        }

        self.check_forbidden_patterns(html, "HTML")?;

        Ok(())
    }

    fn validate_css(&self, css: &str) -> Result<()> {
        if css.len() > self.max_css_size {
            return Err(anyhow!(
                "CSS exceeds maximum size: {} > {}",
                css.len(),
                self.max_css_size
            ));
        }

        if !self.check_balanced_braces(css) {
            return Err(anyhow!("CSS has unbalanced braces"));
        }

        Ok(())
    }

    fn validate_javascript(&self, js: &str) -> Result<()> {
        if js.len() > self.max_js_size {
            return Err(anyhow!(
                "JavaScript exceeds maximum size: {} > {}",
                js.len(),
                self.max_js_size
            ));
        }

        self.check_forbidden_patterns(js, "JavaScript")?;

        Ok(())
    }

    fn check_balanced_tags(&self, html: &str) -> bool {
        let mut stack = Vec::new();
        let self_closing = ["br", "hr", "img", "input", "meta", "link"];

        for tag in html.split('<').skip(1) {
            if let Some(tag_name) = tag.split('>').next() {
                let tag_name = tag_name.trim();

                if let Some(stripped) = tag_name.strip_prefix('/') {
                    let close_tag = stripped.trim();
                    if let Some(open_tag) = stack.pop() {
                        if open_tag != close_tag {
                            return false;
                        }
                    } else {
                        return false;
                    }
                } else if tag_name.ends_with('/') || self_closing.contains(&tag_name) {
                    continue;
                } else {
                    let tag = tag_name.split_whitespace().next().unwrap_or("");
                    if !tag.is_empty() && !tag.starts_with('!') {
                        stack.push(tag.to_string());
                    }
                }
            }
        }

        stack.is_empty()
    }

    fn check_balanced_braces(&self, code: &str) -> bool {
        let mut count = 0;
        for ch in code.chars() {
            match ch {
                '{' => count += 1,
                '}' => {
                    count -= 1;
                    if count < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        count == 0
    }

    fn check_forbidden_patterns(&self, code: &str, code_type: &str) -> Result<()> {
        for pattern in &self.forbidden_patterns {
            if code.contains(pattern) {
                return Err(anyhow!(
                    "{} contains forbidden pattern: {}",
                    code_type,
                    pattern
                ));
            }
        }
        Ok(())
    }
}

impl Default for CodeValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_validation() {
        let validator = CodeValidator::new();

        assert!(validator.validate_html("<div>Test</div>").is_ok());
        assert!(validator.validate_html("<div>Test").is_err());
        assert!(validator.validate_html("<div>Test</span>").is_err());
    }

    #[test]
    fn test_css_validation() {
        let validator = CodeValidator::new();

        assert!(validator.validate_css(".test { color: red; }").is_ok());
        assert!(validator.validate_css(".test { color: red;").is_err());
    }

    #[test]
    fn test_forbidden_patterns() {
        let validator = CodeValidator::new();

        let bad_js = "eval('malicious code')";
        assert!(validator.validate_javascript(bad_js).is_err());

        let good_js = "console.log('hello')";
        assert!(validator.validate_javascript(good_js).is_ok());
    }
}
