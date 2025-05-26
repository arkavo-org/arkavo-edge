use crate::{Result, TestError};
use gherkin::{Feature as GherkinFeature, GherkinEnv, ParseFileError};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feature {
    pub name: String,
    pub description: Option<String>,
    pub background: Option<Background>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Background {
    pub name: Option<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub tags: Vec<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub keyword: StepKeyword,
    pub text: String,
    pub data_table: Option<DataTable>,
    pub doc_string: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StepKeyword {
    Given,
    When,
    Then,
    And,
    But,
}

impl StepKeyword {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "Given" => Some(StepKeyword::Given),
            "When" => Some(StepKeyword::When),
            "Then" => Some(StepKeyword::Then),
            "And" => Some(StepKeyword::And),
            "But" => Some(StepKeyword::But),
            _ => None,
        }
    }
}

impl std::fmt::Display for StepKeyword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepKeyword::Given => write!(f, "Given"),
            StepKeyword::When => write!(f, "When"),
            StepKeyword::Then => write!(f, "Then"),
            StepKeyword::And => write!(f, "And"),
            StepKeyword::But => write!(f, "But"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub struct Parser;

impl Parser {
    pub fn parse_feature_file(path: &Path) -> Result<Feature> {
        let env = GherkinEnv::default();
        let gherkin_feature = GherkinFeature::parse_path(path, env).map_err(|e| match e {
            ParseFileError::Reading { path: _, source } => TestError::Io(source),
            ParseFileError::Parsing {
                path,
                error: _,
                source: _,
            } => TestError::GherkinParse(format!("Parse error in file: {:?}", path)),
        })?;

        Self::convert_feature(&gherkin_feature)
    }

    pub fn parse_feature(content: &str) -> Result<Feature> {
        let env = GherkinEnv::default();
        let gherkin_feature = GherkinFeature::parse(content, env)
            .map_err(|_e| TestError::GherkinParse("Parse error in gherkin content".to_string()))?;

        Self::convert_feature(&gherkin_feature)
    }

    fn convert_feature(gherkin_feature: &GherkinFeature) -> Result<Feature> {
        let background = gherkin_feature.background.as_ref().map(|bg| Background {
            name: if bg.name.is_empty() {
                None
            } else {
                Some(bg.name.clone())
            },
            steps: bg
                .steps
                .iter()
                .map(|step| Step {
                    keyword: StepKeyword::parse(&step.keyword).unwrap_or(StepKeyword::Given),
                    text: step.value.clone(),
                    data_table: step.table.as_ref().map(|table| {
                        let headers = table
                            .rows
                            .first()
                            .map(|row| row.to_vec())
                            .unwrap_or_default();
                        let rows = table.rows[1..].iter().map(|row| row.to_vec()).collect();
                        DataTable { headers, rows }
                    }),
                    doc_string: step.docstring.clone(),
                })
                .collect(),
        });

        let scenarios = gherkin_feature
            .scenarios
            .iter()
            .map(|scenario| Scenario {
                name: scenario.name.clone(),
                tags: scenario.tags.to_vec(),
                steps: scenario
                    .steps
                    .iter()
                    .map(|step| Step {
                        keyword: StepKeyword::parse(&step.keyword).unwrap_or(StepKeyword::Given),
                        text: step.value.clone(),
                        data_table: step.table.as_ref().map(|table| {
                            let headers = table
                                .rows
                                .first()
                                .map(|row| row.to_vec())
                                .unwrap_or_default();
                            let rows = table.rows[1..].iter().map(|row| row.to_vec()).collect();
                            DataTable { headers, rows }
                        }),
                        doc_string: step.docstring.clone(),
                    })
                    .collect(),
            })
            .collect();

        Ok(Feature {
            name: gherkin_feature.name.clone(),
            description: gherkin_feature.description.clone(),
            background,
            scenarios,
        })
    }
}
