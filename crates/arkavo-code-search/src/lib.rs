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
