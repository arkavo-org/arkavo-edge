use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum QwenModel {
    Qwen3_235bA22b,
    #[default]
    Qwen3_32b,
    Qwen3_30bA3b,
    Qwen3_14b,
    Qwen3_8b,
    Qwen3_4b,
    Qwen3_1_7b,
    Qwen3_0_6b,
    QwenMax,
    QwenPlus,
    QwenTurbo,
    QwenVlMax,
    QwenVlMaxLatest,
    QwenVlPlus,
    QwenVlPlusLatest,
    Qwen3VlPlus,
    Qwen3Vl235bA22bThinking,
    Qwen3Vl235bA22bInstruct,
    Custom(String),
}

impl QwenModel {
    pub fn as_str(&self) -> &str {
        match self {
            QwenModel::Qwen3_235bA22b => "qwen3-235b-a22b",
            QwenModel::Qwen3_32b => "qwen3-32b",
            QwenModel::Qwen3_30bA3b => "qwen3-30b-a3b",
            QwenModel::Qwen3_14b => "qwen3-14b",
            QwenModel::Qwen3_8b => "qwen3-8b",
            QwenModel::Qwen3_4b => "qwen3-4b",
            QwenModel::Qwen3_1_7b => "qwen3-1.7b",
            QwenModel::Qwen3_0_6b => "qwen3-0.6b",
            QwenModel::QwenMax => "qwen-max",
            QwenModel::QwenPlus => "qwen-plus",
            QwenModel::QwenTurbo => "qwen-turbo",
            QwenModel::QwenVlMax => "qwen-vl-max",
            QwenModel::QwenVlMaxLatest => "qwen-vl-max-latest",
            QwenModel::QwenVlPlus => "qwen-vl-plus",
            QwenModel::QwenVlPlusLatest => "qwen-vl-plus-latest",
            QwenModel::Qwen3VlPlus => "qwen3-vl-plus",
            QwenModel::Qwen3Vl235bA22bThinking => "qwen3-vl-235b-a22b-thinking",
            QwenModel::Qwen3Vl235bA22bInstruct => "qwen3-vl-235b-a22b-instruct",
            QwenModel::Custom(s) => s.as_str(),
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "qwen3-235b-a22b" => QwenModel::Qwen3_235bA22b,
            "qwen3-32b" => QwenModel::Qwen3_32b,
            "qwen3-30b-a3b" => QwenModel::Qwen3_30bA3b,
            "qwen3-14b" => QwenModel::Qwen3_14b,
            "qwen3-8b" => QwenModel::Qwen3_8b,
            "qwen3-4b" => QwenModel::Qwen3_4b,
            "qwen3-1.7b" => QwenModel::Qwen3_1_7b,
            "qwen3-0.6b" => QwenModel::Qwen3_0_6b,
            "qwen-max" => QwenModel::QwenMax,
            "qwen-plus" => QwenModel::QwenPlus,
            "qwen-turbo" => QwenModel::QwenTurbo,
            "qwen-vl-max" => QwenModel::QwenVlMax,
            "qwen-vl-max-latest" => QwenModel::QwenVlMaxLatest,
            "qwen-vl-plus" => QwenModel::QwenVlPlus,
            "qwen-vl-plus-latest" => QwenModel::QwenVlPlusLatest,
            "qwen3-vl-plus" => QwenModel::Qwen3VlPlus,
            "qwen3-vl-235b-a22b-thinking" => QwenModel::Qwen3Vl235bA22bThinking,
            "qwen3-vl-235b-a22b-instruct" => QwenModel::Qwen3Vl235bA22bInstruct,
            other => QwenModel::Custom(other.to_string()),
        }
    }

    pub fn supports_vision(&self) -> bool {
        matches!(
            self,
            QwenModel::QwenVlMax
                | QwenModel::QwenVlMaxLatest
                | QwenModel::QwenVlPlus
                | QwenModel::QwenVlPlusLatest
                | QwenModel::Qwen3VlPlus
                | QwenModel::Qwen3Vl235bA22bThinking
                | QwenModel::Qwen3Vl235bA22bInstruct
        )
    }

    pub fn supports_tools(&self) -> bool {
        !matches!(
            self,
            QwenModel::Qwen3Vl235bA22bThinking | QwenModel::Custom(_)
        )
    }

    pub fn supports_thinking(&self) -> bool {
        matches!(
            self,
            QwenModel::Qwen3Vl235bA22bThinking | QwenModel::Qwen3VlPlus
        )
    }

    pub fn is_text_only(&self) -> bool {
        !self.supports_vision()
    }
}

impl std::fmt::Display for QwenModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_serialization() {
        assert_eq!(QwenModel::Qwen3_32b.as_str(), "qwen3-32b");
        assert_eq!(QwenModel::QwenVlMaxLatest.as_str(), "qwen-vl-max-latest");
        assert_eq!(
            QwenModel::Qwen3Vl235bA22bThinking.as_str(),
            "qwen3-vl-235b-a22b-thinking"
        );
    }

    #[test]
    fn test_model_parsing() {
        assert_eq!(QwenModel::parse("qwen3-32b"), QwenModel::Qwen3_32b);
        assert_eq!(
            QwenModel::parse("qwen-vl-max-latest"),
            QwenModel::QwenVlMaxLatest
        );
        assert_eq!(
            QwenModel::parse("custom-model"),
            QwenModel::Custom("custom-model".to_string())
        );
    }

    #[test]
    fn test_vision_support() {
        assert!(QwenModel::QwenVlMax.supports_vision());
        assert!(QwenModel::Qwen3VlPlus.supports_vision());
        assert!(!QwenModel::Qwen3_32b.supports_vision());
        assert!(!QwenModel::QwenMax.supports_vision());
    }

    #[test]
    fn test_tool_support() {
        assert!(QwenModel::Qwen3_32b.supports_tools());
        assert!(QwenModel::QwenMax.supports_tools());
        assert!(!QwenModel::Qwen3Vl235bA22bThinking.supports_tools());
    }

    #[test]
    fn test_thinking_support() {
        assert!(QwenModel::Qwen3Vl235bA22bThinking.supports_thinking());
        assert!(QwenModel::Qwen3VlPlus.supports_thinking());
        assert!(!QwenModel::Qwen3_32b.supports_thinking());
    }

    #[test]
    fn test_default_model() {
        assert_eq!(QwenModel::default(), QwenModel::Qwen3_32b);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", QwenModel::Qwen3_14b), "qwen3-14b");
        assert_eq!(format!("{}", QwenModel::QwenVlMax), "qwen-vl-max");
    }
}
