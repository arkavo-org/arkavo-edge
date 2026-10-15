mod codegrep;
mod comby;
mod error;
mod treesitter;

pub use codegrep::CodeGrepTool;
pub use comby::CombyTool;
pub use error::{CodeSearchError, Result};
pub use treesitter::TreeSitterTool;

pub fn register_tools(registry: &mut dyn ToolRegistry) {
    registry.register(Box::new(CodeGrepTool::new()));
    registry.register(Box::new(CombyTool::new()));
    registry.register(Box::new(TreeSitterTool::new()));
}

pub trait ToolRegistry {
    fn register(&mut self, tool: Box<dyn arkavo_mcp::Tool>);
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[derive(Default)]
    struct RecordingRegistry {
        tools: Vec<Box<dyn arkavo_mcp::Tool>>,
    }

    impl ToolRegistry for RecordingRegistry {
        fn register(&mut self, tool: Box<dyn arkavo_mcp::Tool>) {
            self.tools.push(tool);
        }
    }

    #[spec("CS-001")]
    #[test]
    fn register_tools_registers_grep_comby_and_treesitter() {
        let mut registry = RecordingRegistry::default();
        register_tools(&mut registry);
        let names: Vec<&str> = registry
            .tools
            .iter()
            .map(|t| t.schema().name.as_str())
            .collect();
        assert_eq!(
            names,
            ["codegrep_search", "struct_find_replace", "syntax_tree"]
        );
    }

    #[spec("CS-001")]
    #[test]
    fn registered_tools_expose_invocable_schemas() {
        let mut registry = RecordingRegistry::default();
        register_tools(&mut registry);
        assert_eq!(registry.tools.len(), 3);
        for tool in &registry.tools {
            let schema = tool.schema();
            assert!(!schema.description.is_empty(), "{}", schema.name);
            let required = schema.parameters["required"]
                .as_array()
                .unwrap_or_else(|| panic!("{} declares no required params", schema.name));
            assert!(!required.is_empty(), "{}", schema.name);
            for param in required {
                let name = param.as_str().unwrap();
                assert!(
                    schema.parameters["properties"].get(name).is_some(),
                    "{}: required param `{name}` missing from properties",
                    schema.name
                );
            }
        }
    }
}
