use std::time::Duration;

use serde_json::{Value, json};

use crate::ai::{AiError, ApiContent, ApiMessage, ApiRole, ToolCall, ToolDefinition};

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
}
