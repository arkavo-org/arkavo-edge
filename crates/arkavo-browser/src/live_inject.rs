use crate::{BrowserError, Result};
use chromiumoxide::browser::Browser;
use chromiumoxide::page::Page;
use serde_json::json;

pub struct LiveInjector {
    page: Page,
}

impl LiveInjector {
    pub fn new(page: Page) -> Self {
        Self { page }
    }

    pub async fn inject_html(&self, html: &str) -> Result<()> {
        let script = format!(
            r#"
            (function() {{
                const container = document.querySelector('#dynamic-ui-container') ||
                                document.createElement('div');
                container.id = 'dynamic-ui-container';
                container.innerHTML = `{}`;
                if (!document.body.contains(container)) {{
                    document.body.appendChild(container);
                }}
            }})();
            "#,
            html.replace('`', "\\`")
        );

        self.page
            .evaluate(script)
            .await
            .map_err(|e| BrowserError::Playwright(format!("Failed to inject HTML: {e}")))?;

        Ok(())
    }

    pub async fn inject_css(&self, css: &str) -> Result<()> {
        let script = format!(
            r#"
            (function() {{
                let style = document.querySelector('#dynamic-ui-styles');
                if (!style) {{
                    style = document.createElement('style');
                    style.id = 'dynamic-ui-styles';
                    document.head.appendChild(style);
                }}
                style.textContent = `{}`;
            }})();
            "#,
            css.replace('`', "\\`")
        );

        self.page
            .evaluate(script)
            .await
            .map_err(|e| BrowserError::Playwright(format!("Failed to inject CSS: {e}")))?;

        Ok(())
    }

    pub async fn inject_javascript(&self, js: &str) -> Result<()> {
        self.page
            .evaluate(js.to_string())
            .await
            .map_err(|e| BrowserError::Playwright(format!("Failed to inject JavaScript: {e}")))?;

        Ok(())
    }

    pub async fn inject_complete_update(
        &self,
        html: Option<&str>,
        css: Option<&str>,
        js: Option<&str>,
    ) -> Result<()> {
        if let Some(css_code) = css {
            self.inject_css(css_code).await?;
        }

        if let Some(html_code) = html {
            self.inject_html(html_code).await?;
        }

        if let Some(js_code) = js {
            self.inject_javascript(js_code).await?;
        }

        Ok(())
    }

    pub async fn capture_dom_state(&self) -> Result<String> {
        let result = self
            .page
            .evaluate(
                r#"
                (function() {
                    const container = document.querySelector('#dynamic-ui-container');
                    return container ? container.innerHTML : '';
                })();
                "#,
            )
            .await
            .map_err(|e| BrowserError::Playwright(format!("Failed to capture DOM: {e}")))?;

        // EvaluationResult - just return a debug representation
        Ok(format!("{:?}", result))
    }

    pub async fn monitor_changes(&self) -> Result<()> {
        let script = r#"
        (function() {
            if (window.__arkavoChangeMonitor) return;

            const observer = new MutationObserver((mutations) => {
                const changes = mutations.map(m => ({
                    type: m.type,
                    target: m.target.tagName,
                    added: m.addedNodes.length,
                    removed: m.removedNodes.length
                }));

                window.__arkavoChanges = window.__arkavoChanges || [];
                window.__arkavoChanges.push(...changes);
            });

            const container = document.querySelector('#dynamic-ui-container');
            if (container) {
                observer.observe(container, {
                    childList: true,
                    subtree: true,
                    attributes: true,
                    attributeOldValue: true
                });
            }

            window.__arkavoChangeMonitor = observer;
        })();
        "#;

        self.page
            .evaluate(script)
            .await
            .map_err(|e| BrowserError::Playwright(format!("Failed to setup monitoring: {e}")))?;

        Ok(())
    }

    pub async fn get_captured_changes(&self) -> Result<serde_json::Value> {
        let _result = self
            .page
            .evaluate(
                r#"
                (function() {
                    const changes = window.__arkavoChanges || [];
                    window.__arkavoChanges = [];
                    return changes;
                })();
                "#,
            )
            .await
            .map_err(|e| BrowserError::Playwright(format!("Failed to get changes: {e}")))?;

        // For now, return empty array - would need to properly parse EvaluationResult
        Ok(json!([]))
    }
}

pub async fn create_injector_for_browser(browser: &Browser) -> Result<LiveInjector> {
    let page = browser
        .new_page("about:blank")
        .await
        .map_err(|e| BrowserError::Playwright(format!("Failed to create page: {e}")))?;

    Ok(LiveInjector::new(page))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    #[test]
    fn test_injector_creation() {
        // Placeholder test
    }
}
