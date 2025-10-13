use crate::CodeUpdate;
use similar::{ChangeTag, TextDiff};

pub fn calculate_diff_summary(old: &CodeUpdate, new: &CodeUpdate) -> String {
    let mut changes = Vec::new();

    if let (Some(old_html), Some(new_html)) = (&old.html, &new.html)
        && old_html != new_html
    {
        let stats = count_changes(old_html, new_html);
        changes.push(format!(
            "HTML: +{} -{} lines",
            stats.additions, stats.deletions
        ));
    }

    if let (Some(old_css), Some(new_css)) = (&old.css, &new.css)
        && old_css != new_css
    {
        let stats = count_changes(old_css, new_css);
        changes.push(format!(
            "CSS: +{} -{} lines",
            stats.additions, stats.deletions
        ));
    }

    if let (Some(old_js), Some(new_js)) = (&old.javascript, &new.javascript)
        && old_js != new_js
    {
        let stats = count_changes(old_js, new_js);
        changes.push(format!(
            "JS: +{} -{} lines",
            stats.additions, stats.deletions
        ));
    }

    if changes.is_empty() {
        "No changes".to_string()
    } else {
        changes.join(", ")
    }
}

struct DiffStats {
    additions: usize,
    deletions: usize,
}

fn count_changes(old: &str, new: &str) -> DiffStats {
    let diff = TextDiff::from_lines(old, new);
    let mut additions = 0;
    let mut deletions = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => additions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }

    DiffStats {
        additions,
        deletions,
    }
}

pub fn generate_unified_diff(old: &str, new: &str, context: usize) -> String {
    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .context_radius(context)
        .header("old", "new")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_calculation() {
        let old_code = "line1\nline2\nline3";
        let new_code = "line1\nmodified\nline3\nline4";

        let stats = count_changes(old_code, new_code);
        assert_eq!(stats.additions, 3);
        assert_eq!(stats.deletions, 2);
    }

    #[test]
    fn test_unified_diff() {
        let old_code = "console.log('old');";
        let new_code = "console.log('new');";

        let diff = generate_unified_diff(old_code, new_code, 3);
        assert!(diff.contains("old"));
        assert!(diff.contains("new"));
    }

    #[test]
    fn test_diff_summary() {
        let old_update = CodeUpdate {
            id: "1".to_string(),
            html: Some("<div>old</div>".to_string()),
            css: Some(".old {}".to_string()),
            javascript: None,
            timestamp: chrono::Utc::now(),
            source: crate::UpdateSource::LlmGenerated,
        };

        let new_update = CodeUpdate {
            id: "2".to_string(),
            html: Some("<div>new</div>".to_string()),
            css: Some(".new {}".to_string()),
            javascript: Some("console.log('test');".to_string()),
            timestamp: chrono::Utc::now(),
            source: crate::UpdateSource::BrowserEdited,
        };

        let summary = calculate_diff_summary(&old_update, &new_update);
        assert!(summary.contains("HTML"));
        assert!(summary.contains("CSS"));
    }
}
