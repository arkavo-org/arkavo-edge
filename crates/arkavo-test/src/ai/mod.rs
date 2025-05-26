pub mod claude_client;
pub mod planner;
pub mod analysis_engine;

pub use analysis_engine::{
    AnalysisEngine, 
    CodeContext, 
    DomainAnalysis, 
    Property, 
    PropertyCategory,
    TestCase,
    BugAnalysis,
    Severity,
};