use std::time::Duration;

use serde_json::{Value, json};

use crate::ai::{
    AiError, AiResponse, ApiContent, ApiMessage, ApiRole, RoundResult, ToolCall, ToolDefinition,
};

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
            "{persona}\nEntity: {entity_name}\n{context_contents}\n\
             Max 3 paragraphs unless asked for more. End every response with:\n\
             SUMMARY: [one sentence summary]"
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
    ///
    /// When `use_cache` is true, the system prompt is wrapped in a content-block
    /// array with `cache_control: {"type": "ephemeral"}` and the last tool
    /// definition also receives that cache_control field.  Set `use_cache` to
    /// false for one-off calls (/compact, bank detection, /match).
    pub fn build_request_payload(
        &self,
        system: &str,
        messages: &[ApiMessage],
        tools_opt: Option<&[ToolDefinition]>,
        use_cache: bool,
    ) -> Result<String, AiError> {
        let system_value = if use_cache {
            json!([{
                "type": "text",
                "text": system,
                "cache_control": {"type": "ephemeral"}
            }])
        } else {
            json!(system)
        };

        let mut payload = json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system_value,
            "messages": serialize_messages(messages),
        });

        if let Some(tools) = tools_opt.filter(|t| !t.is_empty()) {
            payload["tools"] = json!(serialize_tools(tools, use_cache));
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
        use_cache: bool,
    ) -> Result<Value, AiError> {
        let body = self.build_request_payload(system, messages, tools_opt, use_cache)?;

        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();

        let result = agent
            .post("https://api.anthropic.com/v1/messages")
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("anthropic-beta", "prompt-caching-2024-07-31")
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
    /// Never uses prompt caching.
    pub fn send_simple(&self, system: &str, messages: &[ApiMessage]) -> Result<String, AiError> {
        let response = self.send_request(system, messages, None, false)?;
        let (text, _tools) = Self::parse_response(&response)?;
        text.ok_or_else(|| AiError::ParseError("Response contained no text content".to_string()))
    }

    /// Make a single API call with tool definitions.
    ///
    /// Returns [`RoundResult::Done`] if Claude produced a text-only response,
    /// or [`RoundResult::NeedsToolCall`] if tools must be fulfilled before
    /// continuing.  The caller drives the loop, logging and rendering between
    /// rounds.
    ///
    /// Set `use_cache` to true for chat panel and Pass 2 batch calls; false for
    /// one-off calls like `/match`.
    pub fn send_single_round(
        &self,
        system: &str,
        messages: &[ApiMessage],
        tools: &[ToolDefinition],
        accumulated_text: Option<String>,
        use_cache: bool,
    ) -> Result<RoundResult, AiError> {
        let response = self.send_request(system, messages, Some(tools), use_cache)?;
        classify_round(&response, messages, accumulated_text)
    }
}

// ── Round Classification ──────────────────────────────────────────────────────

/// Classify a single API response into a `RoundResult`.
///
/// Accumulates any text from this round with `accumulated_text` from previous
/// rounds.  If no tool calls are present the full text is parsed for a SUMMARY
/// line and returned as `Done`.  Otherwise `NeedsToolCall` is returned with the
/// assistant's tool_use turn already appended to the message history.
pub(crate) fn classify_round(
    response: &Value,
    messages: &[ApiMessage],
    accumulated_text: Option<String>,
) -> Result<RoundResult, AiError> {
    let (text_block, tool_calls) = AiClient::parse_response(response)?;

    // Accumulate any text content from this round.
    let acc = match (accumulated_text, text_block) {
        (Some(mut existing), Some(new)) => {
            existing.push('\n');
            existing.push_str(&new);
            Some(existing)
        }
        (None, some) => some,
        (some, None) => some,
    };

    if tool_calls.is_empty() {
        // Final text response — no more tool calls.
        let full_text = acc.unwrap_or_default();
        let (content, summary) = AiClient::parse_summary(&full_text);
        Ok(RoundResult::Done(AiResponse::Text { content, summary }))
    } else {
        // Claude wants to call tools — append the assistant turn to history.
        let mut updated = messages.to_vec();
        updated.push(ApiMessage {
            role: ApiRole::Assistant,
            content: ApiContent::ToolUse(tool_calls.clone()),
        });
        Ok(RoundResult::NeedsToolCall {
            tool_calls,
            messages: updated,
            accumulated_text: acc,
        })
    }
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

fn serialize_tools(tools: &[ToolDefinition], use_cache: bool) -> Vec<Value> {
    let last_idx = tools.len().saturating_sub(1);
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if use_cache && i == last_idx {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                    "cache_control": {"type": "ephemeral"},
                })
            } else {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            }
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
    use crate::ai::{ApiContent, ApiRole, RoundResult, ToolDefinition};

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
            .build_request_payload("System prompt", &messages, None, false)
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
            .build_request_payload("sys", &messages, None, false)
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
            .build_request_payload("sys", &[], Some(&tools), false)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();

        assert!(json["tools"].is_array());
        assert_eq!(json["tools"][0]["name"], "get_account");
        assert_eq!(json["tools"][0]["description"], "Look up an account");
    }

    #[test]
    fn payload_omits_tools_key_when_empty() {
        let client = make_client();
        let raw = client
            .build_request_payload("sys", &[], Some(&[]), false)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        // tools key should not be present for empty slice.
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn payload_omits_tools_key_when_none() {
        let client = make_client();
        let raw = client
            .build_request_payload("sys", &[], None, false)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn payload_with_cache_wraps_system_in_content_block() {
        let client = make_client();
        let raw = client
            .build_request_payload("The system prompt", &[], None, true)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        // system should be an array with a single text block.
        let sys = &json["system"];
        assert!(sys.is_array(), "system should be array when use_cache=true");
        assert_eq!(sys[0]["type"], "text");
        assert_eq!(sys[0]["text"], "The system prompt");
        assert_eq!(sys[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn payload_with_cache_marks_last_tool() {
        let client = make_client();
        let tools = vec![
            ToolDefinition {
                name: "first_tool".to_string(),
                description: "First".to_string(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            ToolDefinition {
                name: "last_tool".to_string(),
                description: "Last".to_string(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
        ];
        let raw = client
            .build_request_payload("sys", &[], Some(&tools), true)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        // Only the last tool should have cache_control.
        assert!(
            json["tools"][0].get("cache_control").is_none(),
            "first tool should not have cache_control"
        );
        assert_eq!(
            json["tools"][1]["cache_control"]["type"], "ephemeral",
            "last tool should have cache_control"
        );
    }

    #[test]
    fn payload_without_cache_system_is_string() {
        let client = make_client();
        let raw = client
            .build_request_payload("plain system", &[], None, false)
            .unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["system"], "plain system");
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

    // ── classify_round ────────────────────────────────────────────────────

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
    fn classify_round_text_response_returns_done() {
        let response = text_response("The answer is 42.\n\nSUMMARY: Answered question.");
        let result = classify_round(&response, &[], None).unwrap();
        match result {
            RoundResult::Done(AiResponse::Text { content, summary }) => {
                assert!(content.contains("The answer is 42."));
                assert_eq!(summary, "Answered question.");
            }
            _ => panic!("Expected Done with Text"),
        }
    }

    #[test]
    fn classify_round_tool_use_returns_needs_tool_call() {
        let response = tool_response("tc_1", "get_account", json!({"query": "1000"}));
        let initial_msgs = vec![user_text("What is account 1000?")];
        let result = classify_round(&response, &initial_msgs, None).unwrap();
        match result {
            RoundResult::NeedsToolCall {
                tool_calls,
                messages,
                accumulated_text,
            } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "get_account");
                // Messages should include original + assistant tool_use turn.
                assert_eq!(messages.len(), 2);
                assert!(matches!(messages[1].content, ApiContent::ToolUse(_)));
                assert!(accumulated_text.is_none());
            }
            _ => panic!("Expected NeedsToolCall"),
        }
    }

    #[test]
    fn classify_round_mixed_response_returns_needs_tool_call_with_partial_text() {
        let response = json!({
            "content": [
                {"type": "text", "text": "Let me look that up."},
                {"type": "tool_use", "id": "tc_1", "name": "get_account", "input": {}}
            ],
            "stop_reason": "tool_use"
        });
        let result = classify_round(&response, &[], None).unwrap();
        match result {
            RoundResult::NeedsToolCall {
                tool_calls,
                accumulated_text,
                ..
            } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(accumulated_text, Some("Let me look that up.".to_string()));
            }
            _ => panic!("Expected NeedsToolCall with partial text"),
        }
    }

    #[test]
    fn classify_round_accumulates_text_from_previous_rounds() {
        let response = text_response("Final answer.\n\nSUMMARY: Done.");
        let prior = Some("Earlier text from round 1.".to_string());
        let result = classify_round(&response, &[], prior).unwrap();
        match result {
            RoundResult::Done(AiResponse::Text { content, summary }) => {
                assert!(content.contains("Earlier text from round 1."));
                assert!(content.contains("Final answer."));
                assert_eq!(summary, "Done.");
            }
            _ => panic!("Expected Done with accumulated text"),
        }
    }

    #[test]
    fn classify_round_appends_assistant_turn_to_messages() {
        let response = tool_response("tc_1", "search_accounts", json!({"query": "cash"}));
        let msgs = vec![user_text("Find cash accounts")];
        let result = classify_round(&response, &msgs, None).unwrap();
        match result {
            RoundResult::NeedsToolCall { messages, .. } => {
                assert_eq!(messages.len(), 2);
                assert!(matches!(messages[0].content, ApiContent::Text(_)));
                assert!(matches!(messages[1].content, ApiContent::ToolUse(_)));
            }
            _ => panic!("Expected NeedsToolCall"),
        }
    }

    #[test]
    fn classify_round_parse_error_propagates() {
        let bad_response = json!({"no_content_field": true});
        let result = classify_round(&bad_response, &[], None);
        assert!(matches!(result, Err(AiError::ParseError(_))));
    }
}
