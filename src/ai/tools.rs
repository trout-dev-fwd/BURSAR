use crate::ai::{AiError, ToolCall, ToolDefinition};
use crate::db::EntityDb;

/// Returns all tool definitions for the Claude API request.
/// Fully implemented in Task 4 — stub here to unblock Task 3.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![] // TODO(Task 4): populate all 10 tool definitions
}

/// Dispatch a tool call to the appropriate repo method.
/// Fully implemented in Task 5 — stub returns an error for unknown tools.
pub fn fulfill_tool_call(_tool_call: &ToolCall, _db: &EntityDb) -> Result<String, AiError> {
    // TODO(Task 5): implement per-tool dispatch
    Err(AiError::ParseError(
        "Tool fulfillment not yet implemented".to_string(),
    ))
}
