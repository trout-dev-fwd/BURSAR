pub mod client;
pub mod context;
pub mod csv_import;
pub mod tools;

use crate::types::{ChatRole, MatchConfidence, MatchSource};
use chrono::NaiveDate;

use crate::types::{AccountId, Money};

// ── API Wire Types ────────────────────────────────────────────────────────────

/// A single message in the API conversation history sent to / received from Claude.
#[derive(Debug, Clone)]
pub struct ApiMessage {
    pub role: ApiRole,
    pub content: ApiContent,
}

/// Sender role for the API wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiRole {
    User,
    Assistant,
}

/// Content payload for an API message.
#[derive(Debug, Clone)]
pub enum ApiContent {
    Text(String),
    ToolUse(Vec<ToolCall>),
    ToolResult(Vec<ToolResult>),
}

// ── Tool Types ────────────────────────────────────────────────────────────────

/// A single tool invocation requested by Claude.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Describes a tool for the Claude API `tools` parameter.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// The result of fulfilling a tool call, sent back to Claude.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String, // JSON-serialized result
}

// ── AI Response / Error ───────────────────────────────────────────────────────

/// Parsed result of a Claude API response.
#[derive(Debug)]
pub enum AiResponse {
    Text {
        content: String, // Full display text (SUMMARY line stripped)
        summary: String, // Single-line summary for audit logging
    },
    ToolUse(Vec<ToolCall>),
}

/// Result of a single API round in the tool use loop.
///
/// The caller (App) drives the loop, calling `send_single_round` repeatedly.
/// Between rounds it can log tool calls, update the UI, and fulfill tools
/// without borrow conflicts.
#[derive(Debug)]
pub enum RoundResult {
    /// Final text response — no more tool calls needed.
    Done(AiResponse),
    /// Claude wants to call tools before continuing.
    NeedsToolCall {
        tool_calls: Vec<ToolCall>,
        /// Message history with the assistant's tool_use turn already appended.
        messages: Vec<ApiMessage>,
        /// Any text from this round (rare but possible with mixed responses).
        accumulated_text: Option<String>,
    },
}

/// AI API failure variants.
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("request timed out")]
    Timeout,
    #[error("API error {status}: {body}")]
    ApiError { status: u16, body: String },
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("no API key configured")]
    NoApiKey,
    #[error("maximum tool use depth exceeded")]
    MaxToolDepth,
}

// ── Chat Panel State ──────────────────────────────────────────────────────────

/// A single message displayed in the chat panel.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    pub is_fully_rendered: bool,
}

/// Tracks the state of the typewriter animation for an AI response.
#[derive(Debug, Clone)]
pub struct TypewriterState {
    pub full_text: String,
    /// Byte offset of how many characters have been revealed so far (char-boundary aligned).
    pub display_position: usize,
    /// Index into `ChatPanel.messages` for the message being animated.
    pub message_index: usize,
}

// ── CSV Import Types ──────────────────────────────────────────────────────────

/// A bank statement line normalized to a common format.
#[derive(Debug, Clone)]
pub struct NormalizedTransaction {
    pub date: NaiveDate,
    pub description: String,
    /// Positive = deposit / credit to bank; negative = withdrawal / debit from bank.
    pub amount: Money,
    /// Composite key: `"{bank_name}|{date}|{description}|{amount}"`
    pub import_ref: String,
    /// Original CSV row for debugging.
    pub raw_row: String,
}

/// A proposed mapping of a transaction to a chart-of-accounts entry.
#[derive(Debug, Clone)]
pub struct ImportMatch {
    pub transaction: NormalizedTransaction,
    pub matched_account_id: Option<AccountId>,
    pub matched_account_display: Option<String>,
    pub match_source: MatchSource,
    pub confidence: Option<MatchConfidence>,
    pub reasoning: Option<String>,
    pub rejected: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatRole, MatchConfidence, MatchSource};

    #[test]
    fn tool_call_fields_accessible() {
        let tc = ToolCall {
            id: "tool_123".to_string(),
            name: "get_account".to_string(),
            input: serde_json::json!({"query": "5100"}),
        };
        assert_eq!(tc.id, "tool_123");
        assert_eq!(tc.name, "get_account");
    }

    #[test]
    fn tool_definition_fields_accessible() {
        let td = ToolDefinition {
            name: "get_account".to_string(),
            description: "Look up an account".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        assert_eq!(td.name, "get_account");
    }

    #[test]
    fn ai_response_text_variant() {
        let r = AiResponse::Text {
            content: "The balance is $100.".to_string(),
            summary: "Checked account balance.".to_string(),
        };
        match r {
            AiResponse::Text { content, summary } => {
                assert!(content.contains("$100"));
                assert!(summary.contains("balance"));
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn ai_response_tool_use_variant() {
        let r = AiResponse::ToolUse(vec![ToolCall {
            id: "tc1".to_string(),
            name: "get_account".to_string(),
            input: serde_json::Value::Null,
        }]);
        match r {
            AiResponse::ToolUse(calls) => assert_eq!(calls.len(), 1),
            _ => panic!("expected ToolUse variant"),
        }
    }

    #[test]
    fn chat_message_fields_accessible() {
        let msg = ChatMessage {
            role: ChatRole::Assistant,
            content: "Hello!".to_string(),
            is_fully_rendered: false,
        };
        assert_eq!(msg.role, ChatRole::Assistant);
        assert!(!msg.is_fully_rendered);
    }

    #[test]
    fn typewriter_state_fields_accessible() {
        let ts = TypewriterState {
            full_text: "Hello, world!".to_string(),
            display_position: 0,
            message_index: 2,
        };
        assert_eq!(ts.full_text.len(), 13);
        assert_eq!(ts.display_position, 0);
        assert_eq!(ts.message_index, 2);
    }

    #[test]
    fn import_match_fields_accessible() {
        use crate::types::Money;
        use chrono::NaiveDate;

        let txn = NormalizedTransaction {
            date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            description: "ACME INSURANCE".to_string(),
            amount: Money::from_dollars(100.0),
            import_ref: "TestBank|2026-03-01|ACME INSURANCE|10000000000".to_string(),
            raw_row: "03/01/2026,ACME INSURANCE,100.00".to_string(),
        };
        let im = ImportMatch {
            transaction: txn,
            matched_account_id: None,
            matched_account_display: None,
            match_source: MatchSource::Unmatched,
            confidence: None,
            reasoning: None,
            rejected: false,
        };
        assert!(!im.rejected);
        assert_eq!(im.match_source, MatchSource::Unmatched);
    }

    #[test]
    fn import_match_with_ai_source() {
        use crate::types::Money;
        use chrono::NaiveDate;

        let txn = NormalizedTransaction {
            date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            description: "PAYROLL".to_string(),
            amount: Money::from_dollars(-5000.0),
            import_ref: "TestBank|2026-03-01|PAYROLL|-500000000000".to_string(),
            raw_row: "03/01/2026,PAYROLL,-5000.00".to_string(),
        };
        let im = ImportMatch {
            transaction: txn,
            matched_account_id: Some(AccountId::from(42_i64)),
            matched_account_display: Some("6000 - Salaries".to_string()),
            match_source: MatchSource::Ai,
            confidence: Some(MatchConfidence::High),
            reasoning: Some("Payroll expense account".to_string()),
            rejected: false,
        };
        assert_eq!(im.match_source, MatchSource::Ai);
        assert_eq!(im.confidence, Some(MatchConfidence::High));
    }
}
