use crate::types::{Issue, IssueEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IssueType {
    Bug,
    Feature,
    Documentation,
    Question,
    Maintenance,
    Security,
    Performance,
    Testing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    Trivial,
    Simple,
    Moderate,
    Complex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueAnalysis {
    pub issue_type: IssueType,
    pub complexity: Complexity,
    pub technologies: Vec<String>,
    pub required_capabilities: Vec<String>,
    pub estimated_tokens: u32,
}

impl Complexity {
    pub fn token_budget(&self) -> u32 {
        match self {
            Complexity::Trivial => 10_000,
            Complexity::Simple => 50_000,
            Complexity::Moderate => 200_000,
            Complexity::Complex => 500_000,
        }
    }
}

pub struct IssueAnalyzer;

impl IssueAnalyzer {
    pub fn analyze(event: &IssueEvent) -> IssueAnalysis {
        let issue_type = Self::classify_type(&event.issue);
        let complexity = Self::assess_complexity(&event.issue);
        let technologies = Self::detect_technologies(&event.issue);
        let required_capabilities = Self::determine_capabilities(&issue_type, &technologies);
        let estimated_tokens = complexity.token_budget();

        IssueAnalysis {
            issue_type,
            complexity,
            technologies,
            required_capabilities,
            estimated_tokens,
        }
    }

    fn classify_type(issue: &Issue) -> IssueType {
        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        let title_lower = issue.title.to_lowercase();
        let body_lower = issue.body.as_deref().unwrap_or("").to_lowercase();

        if labels
            .iter()
            .any(|l| l.contains("bug") || l.contains("fix"))
        {
            return IssueType::Bug;
        }

        if labels
            .iter()
            .any(|l| l.contains("security") || l.contains("vulnerability"))
        {
            return IssueType::Security;
        }

        if labels
            .iter()
            .any(|l| l.contains("documentation") || l.contains("docs") || l.contains("readme"))
        {
            return IssueType::Documentation;
        }

        if labels
            .iter()
            .any(|l| l.contains("question") || l.contains("help"))
        {
            return IssueType::Question;
        }

        if labels
            .iter()
            .any(|l| l.contains("performance") || l.contains("optimization") || l.contains("speed"))
        {
            return IssueType::Performance;
        }

        if labels
            .iter()
            .any(|l| l.contains("test") || l.contains("ci"))
        {
            return IssueType::Testing;
        }

        if labels.iter().any(|l| {
            l.contains("enhancement") || l.contains("feature") || l.contains("improvement")
        }) {
            return IssueType::Feature;
        }

        if title_lower.contains("bug") || title_lower.contains("fix") || body_lower.contains("bug")
        {
            return IssueType::Bug;
        }

        if title_lower.contains("feature")
            || title_lower.contains("add")
            || title_lower.contains("implement")
        {
            return IssueType::Feature;
        }

        if title_lower.contains("docs")
            || title_lower.contains("documentation")
            || title_lower.contains("readme")
        {
            return IssueType::Documentation;
        }

        IssueType::Unknown
    }

    fn assess_complexity(issue: &Issue) -> Complexity {
        let mut complexity_score = 0;

        let body_length = issue.body.as_deref().unwrap_or("").len();
        if body_length > 2000 {
            complexity_score += 3;
        } else if body_length > 500 {
            complexity_score += 2;
        } else if body_length > 100 {
            complexity_score += 1;
        }

        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();

        if labels
            .iter()
            .any(|l| l.contains("good first issue") || l.contains("trivial") || l.contains("typo"))
        {
            return Complexity::Trivial;
        }

        if labels
            .iter()
            .any(|l| l.contains("complex") || l.contains("architecture") || l.contains("breaking"))
        {
            complexity_score += 4;
        }

        let title_lower = issue.title.to_lowercase();
        let body_lower = issue.body.as_deref().unwrap_or("").to_lowercase();

        let complex_keywords = [
            "refactor",
            "architecture",
            "redesign",
            "breaking",
            "migration",
            "distributed",
            "scalability",
            "multi-agent",
            "orchestration",
        ];
        for keyword in &complex_keywords {
            if title_lower.contains(keyword) || body_lower.contains(keyword) {
                complexity_score += 2;
            }
        }

        let simple_keywords = ["typo", "update", "version", "dependency", "bump"];
        for keyword in &simple_keywords {
            if title_lower.contains(keyword) {
                complexity_score -= 1;
            }
        }

        if issue.labels.len() > 5 {
            complexity_score += 1;
        }

        if body_lower.contains("```") {
            let code_blocks = body_lower.matches("```").count() / 2;
            complexity_score += code_blocks as i32;
        }

        match complexity_score {
            ..=0 => Complexity::Trivial,
            1..=3 => Complexity::Simple,
            4..=7 => Complexity::Moderate,
            _ => Complexity::Complex,
        }
    }

    fn detect_technologies(issue: &Issue) -> Vec<String> {
        let mut technologies = HashSet::new();

        let text = format!(
            "{} {} {}",
            issue.title,
            issue.body.as_deref().unwrap_or(""),
            issue
                .labels
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();

        let tech_keywords = [
            ("rust", vec!["rust", "cargo", ".rs", "rustc"]),
            ("python", vec!["python", "py", "pip", "pytest"]),
            ("javascript", vec!["javascript", "js", "npm", "node"]),
            ("typescript", vec!["typescript", "ts", ".tsx"]),
            ("docker", vec!["docker", "dockerfile", "container"]),
            ("kubernetes", vec!["kubernetes", "k8s", "kubectl"]),
            ("git", vec!["git", "github", "gitlab"]),
            ("sql", vec!["sql", "sqlite", "postgres", "mysql"]),
            ("llm", vec!["llm", "gpt", "claude", "gemini", "openai"]),
            ("mcp", vec!["mcp", "model context protocol"]),
            ("a2a", vec!["a2a", "agent-to-agent"]),
            ("webhook", vec!["webhook", "github app"]),
            ("axum", vec!["axum", "http server"]),
            ("tokio", vec!["tokio", "async", "await"]),
        ];

        for (tech, keywords) in &tech_keywords {
            if keywords.iter().any(|kw| text.contains(kw)) {
                technologies.insert(tech.to_string());
            }
        }

        let mut tech_vec: Vec<String> = technologies.into_iter().collect();
        tech_vec.sort();
        tech_vec
    }

    fn determine_capabilities(issue_type: &IssueType, technologies: &[String]) -> Vec<String> {
        let mut capabilities = Vec::new();

        match issue_type {
            IssueType::Bug => capabilities.push("debugging".to_string()),
            IssueType::Feature => capabilities.push("feature_development".to_string()),
            IssueType::Documentation => capabilities.push("documentation".to_string()),
            IssueType::Security => {
                capabilities.push("security_analysis".to_string());
                capabilities.push("code_review".to_string());
            }
            IssueType::Performance => {
                capabilities.push("performance_optimization".to_string());
                capabilities.push("profiling".to_string());
            }
            IssueType::Testing => capabilities.push("test_development".to_string()),
            _ => {}
        }

        for tech in technologies {
            capabilities.push(format!("{tech}_expert"));
        }

        capabilities.sort();
        capabilities.dedup();
        capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Label, User};

    fn create_test_issue(title: &str, body: &str, labels: Vec<&str>) -> Issue {
        Issue {
            id: 1,
            number: 1,
            title: title.to_string(),
            body: Some(body.to_string()),
            state: "open".to_string(),
            user: User {
                id: 1,
                login: "test".to_string(),
                user_type: "User".to_string(),
                avatar_url: "".to_string(),
                html_url: "".to_string(),
            },
            labels: labels
                .iter()
                .enumerate()
                .map(|(i, name)| Label {
                    id: i as u64,
                    name: name.to_string(),
                    color: "000000".to_string(),
                    description: None,
                })
                .collect(),
            assignees: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            closed_at: None,
            html_url: "https://github.com/test/test/issues/1".to_string(),
            milestone: None,
        }
    }

    #[test]
    fn test_classify_bug() {
        let issue = create_test_issue("Fix memory leak", "There is a bug", vec!["bug"]);
        let issue_type = IssueAnalyzer::classify_type(&issue);
        assert_eq!(issue_type, IssueType::Bug);
    }

    #[test]
    fn test_classify_documentation() {
        let issue = create_test_issue("Update README", "", vec!["documentation"]);
        let issue_type = IssueAnalyzer::classify_type(&issue);
        assert_eq!(issue_type, IssueType::Documentation);
    }

    #[test]
    fn test_assess_trivial_complexity() {
        let issue = create_test_issue("Fix typo", "Small typo fix", vec!["good first issue"]);
        let complexity = IssueAnalyzer::assess_complexity(&issue);
        assert_eq!(complexity, Complexity::Trivial);
    }

    #[test]
    fn test_detect_rust_technology() {
        let issue = create_test_issue(
            "Add Rust feature",
            "Implement using cargo and rust",
            vec!["rust"],
        );
        let technologies = IssueAnalyzer::detect_technologies(&issue);
        assert!(technologies.contains(&"rust".to_string()));
    }

    #[test]
    fn test_determine_capabilities_for_bug() {
        let technologies = vec!["rust".to_string()];
        let capabilities = IssueAnalyzer::determine_capabilities(&IssueType::Bug, &technologies);
        assert!(capabilities.contains(&"debugging".to_string()));
        assert!(capabilities.contains(&"rust_expert".to_string()));
    }
}
