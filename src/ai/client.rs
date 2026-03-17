use std::time::Duration;

use serde_json::{Value, json};

use crate::ai::{
    AiError, AiResponse, ApiContent, ApiMessage, ApiRole, ToolCall, ToolDefinition, ToolResult,
};
use crate::db::EntityDb;
use crate::types::AiRequestState;

// ── AiClient ──────────────────────────────────────────────────────────────────

/// Stateless Claude API client.  All conversation state is passed in per call.
pub struct AiClient {
    api_key: String,
    model: String,
    timeout: Duration,
}

impl AiClient {
    /// Create a new client with a 10-second timeout.
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            timeout: Duration::from_secs(10),
        }
    }

    // ── System Prompt ──────────────────────────────────────────────────────

    /// Construct the system prompt from config and context.
    pub fn build_system_prompt(persona: &str, entity_name: &str, context_contents: &str) -> String {
        format!(
            "{persona}\n\n\
             You are the AI accountant for **{entity_name}**.\n\n\
             ## Entity Context\n\n\
             {context_contents}\n\n\
             ## Response Instructions\n\n\
             Respond concisely in no more than 3 paragraphs unless more detail is explicitly \
             requested. End every response with exactly this line:\n\n\
             SUMMARY: [one sentence summarising your response]"
        )
    }

    // ── Summary Parsing ────────────────────────────────────────────────────

    /// Split a response into (display_text, summary).
    ///
    /// Looks for the last `SUMMARY:` line.  If missing, falls back to the
    /// first sentence truncated to 100 characters.
    pub fn parse_summary(response_text: &str) -> (String, String) {
        // Search from the right so we find the final SUMMARY: line.
        if let Some(idx) = response_text.rfind("SUMMARY:") {
            let display_text = response_text[..idx].trim_end().to_string();
            let after_marker = response_text[idx + "SUMMARY:".len()..].trim();
            (display_text, after_marker.to_string())
        } else {
            // Fallback: first sentence capped at 100 chars.
            let summary = extract_first_sentence(response_text, 100);
            (response_text.to_string(), summary)
        }
    }

    // ── Payload Construction ───────────────────────────────────────────────

    /// Build and serialise the JSON request payload.
    ///
    /// Exposed as a separate method so tests can inspect the payload without
    /// making a real HTTP call.
    pub fn build_request_payload(
        &self,
        system: &str,
        messages: &[ApiMessage],
        tools_opt: Option<&[ToolDefinition]>,
    ) -> Result<String, AiError> {
        let mut payload = json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": serialize_messages(messages),
        });

        if let Some(tools) = tools_opt.filter(|t| !t.is_empty()) {
            payload["tools"] = json!(serialize_tools(tools));
        }

        serde_json::to_string(&payload)
            .map_err(|e| AiError::ParseError(format!("Failed to serialize request: {e}")))
    }

    // ── Response Parsing ───────────────────────────────────────────────────

    /// Parse a raw API response JSON into (text_block, tool_calls).
    ///
    /// Either the text or the tool_calls vec (or both) may be present.
    pub fn parse_response(response: &Value) -> Result<(Option<String>, Vec<ToolCall>), AiError> {
        let content = response["content"]
            .as_array()
            .ok_or_else(|| AiError::ParseError("Response missing 'content' array".to_string()))?;

        let mut text_block: Option<String> = None;
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for block in content {
            match block["type"].as_str() {
                Some("text") => {
                    let text = block["text"]
                        .as_str()
                        .ok_or_else(|| {
                            AiError::ParseError("Text block missing 'text' field".to_string())
                        })?
                        .to_string();
                    // Accumulate — rare but possible for multiple text blocks.
                    match text_block.as_mut() {
                        Some(existing) => {
                            existing.push('\n');
                            existing.push_str(&text);
                        }
                        None => text_block = Some(text),
                    }
                }
                Some("tool_use") => {
                    let id = block["id"]
                        .as_str()
                        .ok_or_else(|| {
                            AiError::ParseError("tool_use block missing 'id'".to_string())
                        })?
                        .to_string();
                    let name = block["name"]
                        .as_str()
                        .ok_or_else(|| {
                            AiError::ParseError("tool_use block missing 'name'".to_string())
                        })?
                        .to_string();
                    let input = block["input"].clone();
                    tool_calls.push(ToolCall { id, name, input });
                }
                _ => {} // Ignore unknown block types gracefully.
            }
        }

        Ok((text_block, tool_calls))
    }

    // ── HTTP Request ───────────────────────────────────────────────────────

    /// Make the HTTP POST to the Claude API.  Returns the raw JSON response.
    pub(crate) fn send_request(
        &self,
        system: &str,
        messages: &[ApiMessage],
        tools_opt: Option<&[ToolDefinition]>,
    ) -> Result<Value, AiError> {
        let body = self.build_request_payload(system, messages, tools_opt)?;

        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();

        let result = agent
            .post("https://api.anthropic.com/v1/messages")
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("content-type", "application/json")
            .send_string(&body);

        match result {
            Ok(response) => {
                let text = response
                    .into_string()
                    .map_err(|e| AiError::ParseError(format!("Failed to read response: {e}")))?;
                serde_json::from_str(&text)
                    .map_err(|e| AiError::ParseError(format!("Failed to parse response JSON: {e}")))
            }
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                tracing::warn!(status, %body, "Claude API returned HTTP error");
                Err(AiError::ApiError { status, body })
            }
            Err(ureq::Error::Transport(e)) => {
                tracing::warn!(error = %e, "Claude API transport error (timeout or network)");
                Err(AiError::Timeout)
            }
        }
    }

    // ── Public Send Methods ────────────────────────────────────────────────

    /// Send a single message without tool use.  Used for `/compact` and bank
    /// format detection.  Caller decides whether to parse the SUMMARY line.
    pub fn send_simple(&self, system: &str, messages: &[ApiMessage]) -> Result<String, AiError> {
        let response = self.send_request(system, messages, None)?;
        let (text, _tools) = Self::parse_response(&response)?;
        text.ok_or_else(|| AiError::ParseError("Response contained no text content".to_string()))
    }

    /// Send a message with multi-round tool use support.
    ///
    /// Loops up to `max_depth` rounds, calling `on_stage_change` before each
    /// follow-up request so the UI can update.  Returns the final
    /// `AiResponse::Text` once Claude produces a text-only response.
    pub fn send_with_tools(
        &self,
        system: &str,
        messages: &[ApiMessage],
        tools: &[ToolDefinition],
        db: &EntityDb,
        max_depth: usize,
        on_stage_change: &mut dyn FnMut(AiRequestState),
    ) -> Result<AiResponse, AiError> {
        run_tool_loop(
            messages.to_vec(),
            max_depth,
            on_stage_change,
            &mut |msgs| self.send_request(system, msgs, Some(tools)),
            &mut |tool_call| crate::ai::tools::fulfill_tool_call(tool_call, db),
        )
    }
}

// ── Tool Loop ─────────────────────────────────────────────────────────────────

/// Core multi-round tool-use loop.
///
/// Extracted from `AiClient::send_with_tools` so it can be tested without
/// making real HTTP calls.  `make_request` and `fulfill` are injected as
/// closures; production code supplies the real implementations, tests supply
/// mocks.
pub(crate) fn run_tool_loop(
    initial_messages: Vec<ApiMessage>,
    max_depth: usize,
    on_stage_change: &mut dyn FnMut(AiRequestState),
    make_request: &mut dyn FnMut(&[ApiMessage]) -> Result<Value, AiError>,
    fulfill: &mut dyn FnMut(&ToolCall) -> Result<String, AiError>,
) -> Result<AiResponse, AiError> {
    let mut messages = initial_messages;
    let mut accumulated_text: Option<String> = None;

    for round in 0..=max_depth {
        let response = make_request(&messages)?;
        let (text_block, tool_calls) = AiClient::parse_response(&response)?;

        // Accumulate any text content.
        if let Some(text) = text_block {
            match accumulated_text.as_mut() {
                Some(existing) => {
                    existing.push('\n');
                    existing.push_str(&text);
                }
                None => accumulated_text = Some(text),
            }
        }

        if tool_calls.is_empty() {
            // Final text response — we're done.
            let full_text = accumulated_text.unwrap_or_default();
            let (content, summary) = AiClient::parse_summary(&full_text);
            return Ok(AiResponse::Text { content, summary });
        }

        // Max depth reached — return whatever text we have.
        if round == max_depth {
            tracing::warn!(
                "Tool use loop exceeded max depth ({max_depth}); returning partial response"
            );
            let full_text = accumulated_text.unwrap_or_else(|| {
                "I reached the maximum number of tool calls. Please try a simpler question."
                    .to_string()
            });
            let (content, summary) = AiClient::parse_summary(&full_text);
            return Ok(AiResponse::Text { content, summary });
        }

        // Append the assistant's tool_use turn.
        messages.push(ApiMessage {
            role: ApiRole::Assistant,
            content: ApiContent::ToolUse(tool_calls.clone()),
        });

        // Fulfill each tool call and build the user reply.
        let tool_results: Vec<ToolResult> = tool_calls
            .iter()
            .map(|tc| {
                let content = fulfill(tc).unwrap_or_else(|e| {
                    tracing::warn!(tool = %tc.name, error = %e, "Tool fulfillment error");
                    format!("Error: {e}")
                });
                ToolResult {
                    tool_use_id: tc.id.clone(),
                    content,
                }
            })
            .collect();

        messages.push(ApiMessage {
            role: ApiRole::User,
            content: ApiContent::ToolResult(tool_results),
        });

        // Signal that we're entering a follow-up round (not the first call).
        on_stage_change(AiRequestState::FulfillingTools);
    }

    // Unreachable — the loop above always returns before exhausting iterations.
    Err(AiError::MaxToolDepth)
}

// ── Serialisation Helpers ──────────────────────────────────────────────────────

fn serialize_messages(messages: &[ApiMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                ApiRole::User => "user",
                ApiRole::Assistant => "assistant",
            };
            let content = match &msg.content {
                ApiContent::Text(text) => json!(text),
                ApiContent::ToolUse(calls) => json!(
                    calls
                        .iter()
                        .map(|c| json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.name,
                            "input": c.input,
                        }))
                        .collect::<Vec<_>>()
                ),
                ApiContent::ToolResult(results) => json!(
                    results
                        .iter()
                        .map(|r| json!({
                            "type": "tool_result",
                            "tool_use_id": r.tool_use_id,
                            "content": r.content,
                        }))
                        .collect::<Vec<_>>()
                ),
            };
            json!({"role": role, "content": content})
        })
        .collect()
}

fn serialize_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect()
}

// ── Text Utilities ─────────────────────────────────────────────────────────────

/// Extract the first sentence from `text`, capping at `max_chars`.
fn extract_first_sentence(text: &str, max_chars: usize) -> String {
    // Try common sentence-ending punctuation.
    let end = text
        .char_indices()
        .find(|&(_, ch)| ch == '.' || ch == '!' || ch == '?')
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len());

    let sentence = &text[..end];
    if sentence.len() <= max_chars {
        sentence.trim().to_string()
    } else {
        // Truncate at a character boundary.
        let mut boundary = max_chars;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}…", text[..boundary].trim())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ApiContent, ApiRole, ToolDefinition};

    fn make_client() -> AiClient {
        AiClient::new("test-key".to_string(), "claude-test-model".to_string())
    }

    fn user_text(text: &str) -> ApiMessage {
        ApiMessage {
            role: ApiRole::User,
            content: ApiContent::Text(text.to_string()),
        }
    }

    // ── build_system_prompt ────────────────────────────────────────────────

    #[test]
    fn system_prompt_contains_persona() {
        let prompt =
            AiClient::build_system_prompt("Expert CPA", "Acme Corp", "# Notes\nSome context.");
        assert!(prompt.contains("Expert CPA"));
    }

    #[test]
    fn system_prompt_contains_entity_name() {
        let prompt =
            AiClient::build_system_prompt("Expert CPA", "Acme Corp", "# Notes\nSome context.");
        assert!(prompt.contains("Acme Corp"));
    }

    #[test]
    fn system_prompt_contains_context_contents() {
        let prompt =
            AiClient::build_system_prompt("Expert CPA", "Acme Corp", "# Notes\nSome context.");
        assert!(prompt.contains("Some context."));
    }

    #[test]
    fn system_prompt_contains_summary_instruction() {
        let prompt =
            AiClient::build_system_prompt("Expert CPA", "Acme Corp", "# Notes\nSome context.");
        assert!(prompt.contains("SUMMARY:"));
    }

    // ── parse_summary ──────────────────────────────────────────────────────

    #[test]
    fn parse_summary_extracts_summary_line() {
        let text = "The balance is $1,000.\n\nSUMMARY: Checked account balance.";
        let (display, summary) = AiClient::parse_summary(text);
        assert_eq!(display, "The balance is $1,000.");
        assert_eq!(summary, "Checked account balance.");
    }

    #[test]
    fn parse_summary_strips_summary_from_display() {
        let text = "Paragraph one.\n\nParagraph two.\n\nSUMMARY: Short summary.";
        let (display, _) = AiClient::parse_summary(text);
        assert!(!display.contains("SUMMARY:"));
        assert!(display.contains("Paragraph one."));
        assert!(display.contains("Paragraph two."));
    }

    #[test]
    fn parse_summary_fallback_when_no_summary_line() {
        let text = "This is a response with no summary line present here.";
        let (display, summary) = AiClient::parse_summary(text);
        // Display is unchanged.
        assert_eq!(display, text);
        // Summary is a non-empty fallback.
        assert!(!summary.is_empty());
    }

    #[test]
    fn parse_summary_fallback_is_capped_at_100_chars() {
        let long = "A".repeat(200);
        let (_, summary) = AiClient::parse_summary(&long);
        // 100 bytes of content + 3-byte UTF-8 ellipsis = 103 max
        assert!(summary.len() <= 103, "summary len = {}", summary.len());
    }

    #[test]
    fn parse_summary_uses_last_summary_line() {
        // If there are two SUMMARY: lines (unusual), we use the last.
        let text = "SUMMARY: First.\n\nMore text.\n\nSUMMARY: Second.";
        let (_, summary) = AiClient::parse_summary(text);
        assert_eq!(summary, "Second.");
    }

    // ── build_request_payload ──────────────────────────────────────────────

    #[test]
    fn payload_has_required_fields() {
        let client = make_client();
        let messages = vec![user_text("Hello")];
        let raw = client
            .build_request_payload("System prompt", &messages, None)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(json["model"], "claude-test-model");
        assert_eq!(json["max_tokens"], 4096);
        assert_eq!(json["system"], "System prompt");
        assert!(json["messages"].is_array());
    }

    #[test]
    fn payload_text_message_format() {
        let client = make_client();
        let messages = vec![user_text("Hi there")];
        let raw = client
            .build_request_payload("sys", &messages, None)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();

        let msg = &json["messages"][0];
        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"], "Hi there");
    }

    #[test]
    fn payload_includes_tools_when_provided() {
        let client = make_client();
        let tools = vec![ToolDefinition {
            name: "get_account".to_string(),
            description: "Look up an account".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
        }];
        let raw = client
            .build_request_payload("sys", &[], Some(&tools))
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();

        assert!(json["tools"].is_array());
        assert_eq!(json["tools"][0]["name"], "get_account");
        assert_eq!(json["tools"][0]["description"], "Look up an account");
    }

    #[test]
    fn payload_omits_tools_key_when_empty() {
        let client = make_client();
        let raw = client.build_request_payload("sys", &[], Some(&[])).unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        // tools key should not be present for empty slice.
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn payload_omits_tools_key_when_none() {
        let client = make_client();
        let raw = client.build_request_payload("sys", &[], None).unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        assert!(json.get("tools").is_none());
    }

    // ── parse_response ─────────────────────────────────────────────────────

    #[test]
    fn parse_response_extracts_text_block() {
        let response = json!({
            "content": [{"type": "text", "text": "The answer is 42."}],
            "stop_reason": "end_turn"
        });
        let (text, tools) = AiClient::parse_response(&response).unwrap();
        assert_eq!(text, Some("The answer is 42.".to_string()));
        assert!(tools.is_empty());
    }

    #[test]
    fn parse_response_extracts_tool_use_block() {
        let response = json!({
            "content": [{
                "type": "tool_use",
                "id": "toolu_123",
                "name": "get_account",
                "input": {"query": "5100"}
            }],
            "stop_reason": "tool_use"
        });
        let (text, tools) = AiClient::parse_response(&response).unwrap();
        assert!(text.is_none());
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].id, "toolu_123");
        assert_eq!(tools[0].name, "get_account");
        assert_eq!(tools[0].input["query"], "5100");
    }

    #[test]
    fn parse_response_handles_mixed_blocks() {
        let response = json!({
            "content": [
                {"type": "text", "text": "I will look that up."},
                {
                    "type": "tool_use",
                    "id": "toolu_abc",
                    "name": "search_accounts",
                    "input": {"query": "cash"}
                }
            ],
            "stop_reason": "tool_use"
        });
        let (text, tools) = AiClient::parse_response(&response).unwrap();
        assert_eq!(text, Some("I will look that up.".to_string()));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search_accounts");
    }

    #[test]
    fn parse_response_returns_error_on_missing_content() {
        let response = json!({"stop_reason": "end_turn"});
        let result = AiClient::parse_response(&response);
        assert!(matches!(result, Err(AiError::ParseError(_))));
    }

    #[test]
    fn parse_response_ignores_unknown_block_types() {
        let response = json!({
            "content": [
                {"type": "unknown_future_type", "data": "something"},
                {"type": "text", "text": "Hello"}
            ]
        });
        let (text, tools) = AiClient::parse_response(&response).unwrap();
        assert_eq!(text, Some("Hello".to_string()));
        assert!(tools.is_empty());
    }

    // ── serialize_messages helpers ─────────────────────────────────────────

    #[test]
    fn serialize_tool_result_message() {
        use crate::ai::ToolResult;
        let msg = ApiMessage {
            role: ApiRole::User,
            content: ApiContent::ToolResult(vec![ToolResult {
                tool_use_id: "toolu_xyz".to_string(),
                content: r#"{"balance": "$500"}"#.to_string(),
            }]),
        };
        let serialized = serialize_messages(&[msg]);
        let json = &serialized[0];
        assert_eq!(json["role"], "user");
        let content = &json["content"][0];
        assert_eq!(content["type"], "tool_result");
        assert_eq!(content["tool_use_id"], "toolu_xyz");
    }

    #[test]
    fn serialize_tool_use_message() {
        use crate::ai::ToolCall;
        let msg = ApiMessage {
            role: ApiRole::Assistant,
            content: ApiContent::ToolUse(vec![ToolCall {
                id: "toolu_1".to_string(),
                name: "get_account".to_string(),
                input: json!({"account_number": "1000"}),
            }]),
        };
        let serialized = serialize_messages(&[msg]);
        let json = &serialized[0];
        assert_eq!(json["role"], "assistant");
        let block = &json["content"][0];
        assert_eq!(block["type"], "tool_use");
        assert_eq!(block["id"], "toolu_1");
        assert_eq!(block["name"], "get_account");
    }

    // ── extract_first_sentence ─────────────────────────────────────────────

    #[test]
    fn extract_first_sentence_ends_at_period() {
        let result = extract_first_sentence("Hello world. More text.", 100);
        assert_eq!(result, "Hello world.");
    }

    #[test]
    fn extract_first_sentence_truncates_long_text() {
        let long = format!("{}.", "word ".repeat(30));
        let result = extract_first_sentence(&long, 50);
        // 50 bytes of content + 3-byte UTF-8 ellipsis = 53 max
        assert!(result.len() <= 53, "len = {}", result.len());
    }

    #[test]
    fn extract_first_sentence_handles_no_punctuation() {
        let result = extract_first_sentence("No sentence ending here", 100);
        assert_eq!(result, "No sentence ending here");
    }

    // ── run_tool_loop ──────────────────────────────────────────────────────

    /// Build a JSON response with a text block.
    fn text_response(text: &str) -> Value {
        json!({
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn"
        })
    }

    /// Build a JSON response with a single tool_use block.
    fn tool_response(id: &str, name: &str, input: Value) -> Value {
        json!({
            "content": [{
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }],
            "stop_reason": "tool_use"
        })
    }

    #[test]
    fn loop_single_round_text_returns_immediately() {
        let mut responses = vec![text_response(
            "The answer is 42.\n\nSUMMARY: Answered question.",
        )];
        let mut stage_calls: Vec<AiRequestState> = Vec::new();

        let result = run_tool_loop(
            vec![],
            5,
            &mut |s| stage_calls.push(s),
            &mut |_msgs| Ok(responses.remove(0)),
            &mut |_tc| Ok("unused".to_string()),
        );

        let ai_response = result.unwrap();
        match ai_response {
            AiResponse::Text { content, summary } => {
                assert!(content.contains("The answer is 42."));
                assert_eq!(summary, "Answered question.");
            }
            _ => panic!("Expected Text response"),
        }
        // No stage changes for a single text round.
        assert!(stage_calls.is_empty());
    }

    #[test]
    fn loop_tool_then_text_fulfills_and_returns() {
        let mut responses = vec![
            tool_response("tc_1", "get_account", json!({"query": "1000"})),
            text_response("Account 1000 has $500.\n\nSUMMARY: Checked account balance."),
        ];
        let mut fulfilled_tools: Vec<String> = Vec::new();
        let mut stage_calls: Vec<AiRequestState> = Vec::new();

        let result = run_tool_loop(
            vec![],
            5,
            &mut |s| stage_calls.push(s),
            &mut |_msgs| Ok(responses.remove(0)),
            &mut |tc| {
                fulfilled_tools.push(tc.name.clone());
                Ok(r#"{"account_number": "1000", "balance": "$500"}"#.to_string())
            },
        );

        let ai_response = result.unwrap();
        match ai_response {
            AiResponse::Text { content, summary } => {
                assert!(content.contains("Account 1000"));
                assert_eq!(summary, "Checked account balance.");
            }
            _ => panic!("Expected Text response"),
        }
        // Tool was fulfilled.
        assert_eq!(fulfilled_tools, vec!["get_account"]);
        // FulfillingTools called once before the second request.
        assert_eq!(stage_calls.len(), 1);
        assert!(matches!(stage_calls[0], AiRequestState::FulfillingTools));
    }

    #[test]
    fn loop_two_tool_rounds_before_text() {
        let mut responses = vec![
            tool_response("tc_1", "get_account", json!({})),
            tool_response("tc_2", "search_accounts", json!({})),
            text_response("Done.\n\nSUMMARY: Completed two tool calls."),
        ];
        let mut stage_calls: Vec<AiRequestState> = Vec::new();
        let mut fulfill_count = 0usize;

        let result = run_tool_loop(
            vec![],
            5,
            &mut |s| stage_calls.push(s),
            &mut |_msgs| Ok(responses.remove(0)),
            &mut |_tc| {
                fulfill_count += 1;
                Ok("result".to_string())
            },
        );

        assert!(result.is_ok());
        assert_eq!(fulfill_count, 2); // one tool per round
        assert_eq!(stage_calls.len(), 2); // FulfillingTools called twice
    }

    #[test]
    fn loop_max_depth_returns_fallback() {
        // Always respond with a tool_use — never sends text.
        let mut stage_calls: Vec<AiRequestState> = Vec::new();
        let max_depth = 3;

        let result = run_tool_loop(
            vec![],
            max_depth,
            &mut |s| stage_calls.push(s),
            &mut |_msgs| Ok(tool_response("tc_x", "get_account", json!({}))),
            &mut |_tc| Ok("result".to_string()),
        );

        // Returns a fallback text rather than an error.
        match result.unwrap() {
            AiResponse::Text { content, .. } => {
                assert!(!content.is_empty());
            }
            _ => panic!("Expected fallback Text"),
        }
        // Called for rounds 1..=max_depth (not for round 0).
        assert_eq!(stage_calls.len(), max_depth);
    }

    #[test]
    fn loop_timeout_propagates_error() {
        let mut stage_calls: Vec<AiRequestState> = Vec::new();

        let result = run_tool_loop(
            vec![],
            5,
            &mut |s| stage_calls.push(s),
            &mut |_msgs| Err(AiError::Timeout),
            &mut |_tc| Ok("result".to_string()),
        );

        assert!(matches!(result, Err(AiError::Timeout)));
    }

    #[test]
    fn loop_timeout_on_follow_up_request_propagates() {
        let mut call_count = 0usize;
        let mut stage_calls: Vec<AiRequestState> = Vec::new();

        let result = run_tool_loop(
            vec![],
            5,
            &mut |s| stage_calls.push(s),
            &mut |_msgs| {
                call_count += 1;
                if call_count == 1 {
                    Ok(tool_response("tc_1", "get_account", json!({})))
                } else {
                    Err(AiError::Timeout)
                }
            },
            &mut |_tc| Ok("result".to_string()),
        );

        assert!(matches!(result, Err(AiError::Timeout)));
    }

    #[test]
    fn loop_tool_results_appended_in_follow_up() {
        // Inspect the messages passed to the second request.
        let mut received_messages: Vec<Vec<ApiMessage>> = Vec::new();
        let mut responses = vec![
            tool_response("tc_1", "get_account", json!({"query": "1000"})),
            text_response("Done.\n\nSUMMARY: Finished."),
        ];
        let mut stage_calls: Vec<AiRequestState> = Vec::new();

        run_tool_loop(
            vec![],
            5,
            &mut |s| stage_calls.push(s),
            &mut |msgs| {
                received_messages.push(msgs.to_vec());
                Ok(responses.remove(0))
            },
            &mut |_tc| Ok(r#"{"result": "ok"}"#.to_string()),
        )
        .unwrap();

        // Second call should include the assistant tool_use + user tool_result.
        let second_call_msgs = &received_messages[1];
        assert_eq!(second_call_msgs.len(), 2);
        assert!(matches!(
            second_call_msgs[0].content,
            ApiContent::ToolUse(_)
        ));
        assert!(matches!(
            second_call_msgs[1].content,
            ApiContent::ToolResult(_)
        ));
    }

    #[test]
    fn loop_on_stage_change_called_for_each_follow_up_round() {
        let mut responses = vec![
            tool_response("tc_1", "get_account", json!({})),
            tool_response("tc_2", "get_account", json!({})),
            text_response("Final.\n\nSUMMARY: Done."),
        ];
        let mut stage_calls: Vec<AiRequestState> = Vec::new();

        run_tool_loop(
            vec![],
            5,
            &mut |s| stage_calls.push(s),
            &mut |_msgs| Ok(responses.remove(0)),
            &mut |_tc| Ok("r".to_string()),
        )
        .unwrap();

        // on_stage_change called once before each follow-up (rounds 2 and 3 here).
        assert_eq!(stage_calls.len(), 2);
        assert!(
            stage_calls
                .iter()
                .all(|s| matches!(s, AiRequestState::FulfillingTools))
        );
    }
}
