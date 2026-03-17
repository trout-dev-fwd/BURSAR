use serde_json::json;

use crate::ai::{AiError, ToolCall, ToolDefinition};
use crate::db::EntityDb;

// ── Tool Definitions ──────────────────────────────────────────────────────────

/// Returns all 10 tool definitions for the Claude API `tools` parameter.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "get_account".to_string(),
            description: "Look up a specific account by number or name. Returns account details \
                           including type, balance, and whether it is a placeholder. Use this when \
                           the user refers to a specific account number or name."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Account number, name, or partial substring to search for."
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "get_account_children".to_string(),
            description: "Get all direct child accounts under a placeholder (parent) account. \
                           Returns the list of children with their balances. Use this to explore \
                           the chart of accounts hierarchy."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": {
                        "type": "integer",
                        "description": "The database ID of the parent placeholder account."
                    }
                },
                "required": ["account_id"]
            }),
        },
        ToolDefinition {
            name: "search_accounts".to_string(),
            description: "Search accounts by name or number substring. Returns all matching \
                           accounts with their current balances. Use this to find accounts when \
                           you are not sure of the exact number or name."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Substring to search for in account names and numbers."
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "get_gl_transactions".to_string(),
            description: "Get general ledger transactions for a specific account, optionally \
                           filtered by date range. Returns transaction lines with dates, \
                           descriptions, debit/credit amounts, and running balance."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": {
                        "type": "integer",
                        "description": "The database ID of the account to query."
                    },
                    "start_date": {
                        "type": "string",
                        "description": "Optional start date in YYYY-MM-DD format (inclusive)."
                    },
                    "end_date": {
                        "type": "string",
                        "description": "Optional end date in YYYY-MM-DD format (inclusive)."
                    }
                },
                "required": ["account_id"]
            }),
        },
        ToolDefinition {
            name: "get_journal_entry".to_string(),
            description: "Get full details of a journal entry by its number, including all debit \
                           and credit lines, accounts, amounts, memo, status (Draft/Posted), and \
                           date."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "je_number": {
                        "type": "integer",
                        "description": "The journal entry number (JE #)."
                    }
                },
                "required": ["je_number"]
            }),
        },
        ToolDefinition {
            name: "get_open_ar_items".to_string(),
            description: "Get accounts receivable items. Returns invoices, their amounts, due \
                           dates, and current status. Optionally filter by status."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Optional status filter: 'Open', 'Partial', or 'Paid'.",
                        "enum": ["Open", "Partial", "Paid"]
                    }
                }
            }),
        },
        ToolDefinition {
            name: "get_open_ap_items".to_string(),
            description: "Get accounts payable items. Returns bills, their amounts, due dates, \
                           and current status. Optionally filter by status."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Optional status filter: 'Open', 'Partial', or 'Paid'.",
                        "enum": ["Open", "Partial", "Paid"]
                    }
                }
            }),
        },
        ToolDefinition {
            name: "get_envelope_balances".to_string(),
            description:
                "Get all budget envelope allocations and their current available amounts. \
                           Shows envelope name, allocated amount, spent amount, and remaining \
                           balance."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "get_trial_balance".to_string(),
            description:
                "Get a trial balance across all active accounts, showing debit and credit \
                           balances. Optionally specify an as-of date for a historical snapshot."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "as_of_date": {
                        "type": "string",
                        "description": "Optional as-of date in YYYY-MM-DD format. Defaults to today."
                    }
                }
            }),
        },
        ToolDefinition {
            name: "get_audit_log".to_string(),
            description: "Search the audit log for recorded actions. Can filter by action type \
                           (e.g., 'AiPrompt', 'AiResponse', 'JeCreated', 'JePosted') and/or date \
                           range. Use this to review past AI interactions or track changes."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Optional action type filter (e.g., 'AiPrompt', 'JePosted')."
                    },
                    "start_date": {
                        "type": "string",
                        "description": "Optional start date in YYYY-MM-DD format (inclusive)."
                    },
                    "end_date": {
                        "type": "string",
                        "description": "Optional end date in YYYY-MM-DD format (inclusive)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of entries to return. Defaults to 50."
                    }
                }
            }),
        },
    ]
}

// ── Tool Fulfillment ──────────────────────────────────────────────────────────

/// Dispatch a tool call to the appropriate repo method.
/// Implemented in Task 5 — stub returns an error for unknown tools.
pub fn fulfill_tool_call(_tool_call: &ToolCall, _db: &EntityDb) -> Result<String, AiError> {
    // TODO(Task 5): implement per-tool dispatch
    Err(AiError::ParseError(
        "Tool fulfillment not yet implemented".to_string(),
    ))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_definitions_returns_ten_tools() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 10);
    }

    #[test]
    fn all_tool_names_are_unique() {
        let defs = tool_definitions();
        let mut names = std::collections::HashSet::new();
        for def in &defs {
            assert!(names.insert(&def.name), "Duplicate tool name: {}", def.name);
        }
    }

    #[test]
    fn expected_tool_names_present() {
        let defs = tool_definitions();
        let names: std::collections::HashSet<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        let expected = [
            "get_account",
            "get_account_children",
            "search_accounts",
            "get_gl_transactions",
            "get_journal_entry",
            "get_open_ar_items",
            "get_open_ap_items",
            "get_envelope_balances",
            "get_trial_balance",
            "get_audit_log",
        ];
        for name in expected {
            assert!(names.contains(name), "Missing tool: {name}");
        }
    }

    #[test]
    fn each_definition_has_non_empty_description() {
        for def in tool_definitions() {
            assert!(
                !def.description.is_empty(),
                "Tool '{}' has empty description",
                def.name
            );
        }
    }

    #[test]
    fn each_schema_is_valid_json_object() {
        for def in tool_definitions() {
            assert!(
                def.input_schema.is_object(),
                "Tool '{}' schema is not an object",
                def.name
            );
            assert_eq!(
                def.input_schema["type"], "object",
                "Tool '{}' schema missing type:object",
                def.name
            );
        }
    }

    #[test]
    fn required_fields_match_non_optional_params() {
        let defs = tool_definitions();
        let defs_map: std::collections::HashMap<&str, &ToolDefinition> =
            defs.iter().map(|d| (d.name.as_str(), d)).collect();

        // Tools with required params.
        let required_cases: &[(&str, &[&str])] = &[
            ("get_account", &["query"]),
            ("get_account_children", &["account_id"]),
            ("search_accounts", &["query"]),
            ("get_gl_transactions", &["account_id"]),
            ("get_journal_entry", &["je_number"]),
        ];
        for (name, required) in required_cases {
            let schema = &defs_map[*name].input_schema;
            let req = schema["required"]
                .as_array()
                .unwrap_or_else(|| panic!("Tool '{name}' missing 'required' array"));
            let req_strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
            for field in *required {
                assert!(
                    req_strs.contains(field),
                    "Tool '{name}' required array missing '{field}'"
                );
            }
        }

        // Tools with NO required params (optional-only or no params).
        let no_required: &[&str] = &[
            "get_open_ar_items",
            "get_open_ap_items",
            "get_envelope_balances",
            "get_trial_balance",
            "get_audit_log",
        ];
        for name in no_required {
            let schema = &defs_map[*name].input_schema;
            // Either "required" key absent or empty array.
            if let Some(req) = schema.get("required") {
                assert!(
                    req.as_array().map(|a| a.is_empty()).unwrap_or(false),
                    "Tool '{name}' should have no required fields but got: {req}"
                );
            }
        }
    }

    #[test]
    fn all_definitions_serialize_to_valid_json() {
        for def in tool_definitions() {
            let serialized = serde_json::to_string(&def.input_schema);
            assert!(
                serialized.is_ok(),
                "Tool '{}' schema failed to serialize",
                def.name
            );
        }
    }

    #[test]
    fn get_gl_transactions_has_optional_date_fields() {
        let defs = tool_definitions();
        let gl = defs
            .iter()
            .find(|d| d.name == "get_gl_transactions")
            .unwrap();
        let props = &gl.input_schema["properties"];
        assert!(props["start_date"].is_object(), "start_date missing");
        assert!(props["end_date"].is_object(), "end_date missing");
        // Date fields should NOT be in required (they're optional).
        let required = gl.input_schema["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(!req_strs.contains(&"start_date"));
        assert!(!req_strs.contains(&"end_date"));
    }

    #[test]
    fn get_audit_log_has_limit_parameter() {
        let defs = tool_definitions();
        let al = defs.iter().find(|d| d.name == "get_audit_log").unwrap();
        let props = &al.input_schema["properties"];
        assert!(props["limit"].is_object(), "limit param missing");
        assert_eq!(props["limit"]["type"], "integer");
    }
}
