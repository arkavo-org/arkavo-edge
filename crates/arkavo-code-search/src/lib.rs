mod codegrep;
mod error;

pub use codegrep::CodeGrepTool;
pub use error::{CodeSearchError, Result};

pub fn register_tools(registry: &mut dyn ToolRegistry) {
    registry.register(Box::new(CodeGrepTool::new()));
}

pub trait ToolRegistry {
    fn register(&mut self, tool: Box<dyn arkavo_mcp::Tool>);
}
