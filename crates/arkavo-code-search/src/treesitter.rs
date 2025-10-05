use crate::{CodeSearchError, Result};
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;
use streaming_iterator::StreamingIteratorMut;
use tokio::fs;
use tree_sitter::{Language, Parser, Query, QueryCursor};

unsafe extern "C" {
    fn tree_sitter_rust() -> Language;
    fn tree_sitter_python() -> Language;
    fn tree_sitter_javascript() -> Language;
    fn tree_sitter_typescript() -> Language;
    fn tree_sitter_go() -> Language;
}

pub struct TreeSitterTool {
    schema: ToolSchema,
}

impl TreeSitterTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "syntax_tree".to_string(),
                description: "Parse code with tree-sitter for AST analysis and precise edits"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to the file to parse"
                        },
                        "language": {
                            "type": "string",
                            "enum": ["rust", "python", "javascript", "typescript", "go", "auto"],
                            "description": "Programming language (auto-detect from extension)"
                        },
                        "query": {
                            "type": "string",
                            "description": "Tree-sitter query to execute (optional)"
                        },
                        "output_format": {
                            "type": "string",
                            "enum": ["tree", "nodes", "captures"],
                            "description": "Output format: tree (S-expr), nodes (JSON), captures (query results)"
                        }
                    },
                    "required": ["file_path"]
                }),
            },
        }
    }

    fn get_language(lang: &str) -> Result<Language> {
        unsafe {
            match lang {
                "rust" => Ok(tree_sitter_rust()),
                "python" => Ok(tree_sitter_python()),
                "javascript" => Ok(tree_sitter_javascript()),
                "typescript" => Ok(tree_sitter_typescript()),
                "go" => Ok(tree_sitter_go()),
                _ => Err(CodeSearchError::ToolError(format!(
                    "Unsupported language: {lang}"
                ))),
            }
        }
    }

    fn detect_language(file_path: &str) -> Result<&'static str> {
        let ext = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| CodeSearchError::ToolError("No file extension".to_string()))?;

        match ext {
            "rs" => Ok("rust"),
            "py" => Ok("python"),
            "js" => Ok("javascript"),
            "ts" => Ok("typescript"),
            "go" => Ok("go"),
            _ => Err(CodeSearchError::ToolError(format!(
                "Unsupported file extension: {ext}"
            ))),
        }
    }

    async fn parse_file(&self, params: &Value) -> Result<Value> {
        let file_path = params["file_path"]
            .as_str()
            .ok_or_else(|| CodeSearchError::ToolError("Missing file_path".to_string()))?;

        let lang_str = if let Some(lang) = params.get("language").and_then(|v| v.as_str()) {
            if lang == "auto" {
                Self::detect_language(file_path)?
            } else {
                lang
            }
        } else {
            Self::detect_language(file_path)?
        };

        let language = Self::get_language(lang_str)?;
        let source = fs::read_to_string(file_path)
            .await
            .map_err(CodeSearchError::IoError)?;

        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| CodeSearchError::ToolError(format!("Failed to set language: {e}")))?;

        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| CodeSearchError::ToolError("Failed to parse".to_string()))?;

        let output_format = params
            .get("output_format")
            .and_then(|v| v.as_str())
            .unwrap_or("tree");

        match output_format {
            "tree" => Ok(json!({
                "format": "tree",
                "language": lang_str,
                "tree": tree.root_node().to_sexp()
            })),
            "nodes" => {
                let nodes = Self::extract_nodes(tree.root_node(), &source);
                Ok(json!({
                    "format": "nodes",
                    "language": lang_str,
                    "nodes": nodes
                }))
            }
            "captures" => {
                if let Some(query_str) = params.get("query").and_then(|v| v.as_str()) {
                    let query = Query::new(&language, query_str)
                        .map_err(|e| CodeSearchError::ToolError(format!("Invalid query: {e}")))?;

                    let mut cursor = QueryCursor::new();
                    let mut captures_vec = Vec::new();
                    let mut captures_iter =
                        cursor.captures(&query, tree.root_node(), source.as_bytes());

                    while let Some((m, _)) = captures_iter.next_mut() {
                        for c in m.captures {
                            let start = c.node.start_position();
                            let end = c.node.end_position();
                            captures_vec.push(json!({
                                "text": c.node.utf8_text(source.as_bytes()).unwrap_or(""),
                                "kind": c.node.kind(),
                                "start": {"row": start.row, "column": start.column},
                                "end": {"row": end.row, "column": end.column}
                            }));
                        }
                    }

                    Ok(json!({
                        "format": "captures",
                        "language": lang_str,
                        "captures": captures_vec
                    }))
                } else {
                    Err(CodeSearchError::ToolError(
                        "Query required for captures format".to_string(),
                    ))
                }
            }
            _ => Err(CodeSearchError::ToolError(format!(
                "Invalid format: {output_format}"
            ))),
        }
    }

    fn extract_nodes(node: tree_sitter::Node, source: &str) -> Vec<Value> {
        let start = node.start_position();
        let end = node.end_position();

        let mut nodes = vec![json!({
            "kind": node.kind(),
            "start": {"row": start.row, "column": start.column},
            "end": {"row": end.row, "column": end.column},
            "text": node.utf8_text(source.as_bytes()).unwrap_or("").lines().next().unwrap_or("")
        })];

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            nodes.extend(Self::extract_nodes(child, source));
        }

        nodes
    }
}

impl Default for TreeSitterTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TreeSitterTool {
    async fn execute(
        &self,
        params: Value,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.parse_file(&params)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_rust() {
        let tool = TreeSitterTool::new();
        let params = json!({
            "file_path": "src/lib.rs",
            "language": "rust",
            "output_format": "tree"
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok());
    }
}
