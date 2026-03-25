use serde_json::{Value, json};

use crate::ai::{AiError, ToolCall, ToolDefinition};
use crate::db::EntityDb;
use crate::db::journal_repo::DateRange;
use crate::types::{ArApStatus, AuditAction};

// ── Tool Definitions ──────────────────────────────────────────────────────────

/// Returns the `get_tax_tag` tool definition.
///
/// This tool is only included in Tax tab AI requests (`tool_definitions_with_tax`).
pub fn tax_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "get_tax_tag".to_string(),
        description: "Get tax classification for a journal entry by JE number. \
                       Returns form_tag, status, reason, and ai_suggested_form."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "je_number": {
                    "type": "string",
                    "description": "The journal entry number, e.g. 'JE-0004' or '4'."
                }
            },
            "required": ["je_number"]
        }),
    }
}

/// Returns the standard 10 tool definitions for non-Tax tab AI requests.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "get_account".to_string(),
            description: "Look up account by number or name. Returns type, balance, placeholder flag."
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
            description: "Get child accounts under a placeholder account with balances."
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
            description: "Search accounts by name/number substring. Returns matches with balances."
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
            description: "Get GL transactions for an account. Optional date range filter. Returns lines with debit/credit and running balance."
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
            description: "Get journal entry by number: all lines, accounts, amounts, memo, status, date."
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
            description: "Get AR items: amounts, due dates, status. Optional status filter."
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
            description: "Get AP items: amounts, due dates, status. Optional status filter."
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
            description: "Get all envelope allocations with available amounts."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "get_trial_balance".to_string(),
            description: "Trial balance across all accounts. Optional as-of date."
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
            description: "Search audit log by action type and/or date range."
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
/// Returns a JSON string of the result, or an error description (never panics).
pub fn fulfill_tool_call(tool_call: &ToolCall, db: &EntityDb) -> Result<String, AiError> {
    let params = &tool_call.input;
    let result = match tool_call.name.as_str() {
        "get_account" => handle_get_account(params, db),
        "get_account_children" => handle_get_account_children(params, db),
        "search_accounts" => handle_search_accounts(params, db),
        "get_gl_transactions" => handle_get_gl_transactions(params, db),
        "get_journal_entry" => handle_get_journal_entry(params, db),
        "get_open_ar_items" => handle_get_open_ar_items(params, db),
        "get_open_ap_items" => handle_get_open_ap_items(params, db),
        "get_envelope_balances" => handle_get_envelope_balances(db),
        "get_trial_balance" => handle_get_trial_balance(params, db),
        "get_audit_log" => handle_get_audit_log(params, db),
        "get_tax_tag" => handle_get_tax_tag(params, db),
        unknown => {
            return Ok(format!("Error: unknown tool '{unknown}'"));
        }
    };
    result.map_err(|e| AiError::ParseError(format!("Tool error in '{}': {e}", tool_call.name)))
}

// ── Individual Handlers ───────────────────────────────────────────────────────

fn handle_get_account(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    let query = require_string(params, "query")?;
    let accounts = db.accounts().search(query)?;
    if accounts.is_empty() {
        return Ok(format!(r#"{{"found": false, "query": "{query}"}}"#));
    }
    let account = &accounts[0];
    let balance = db.accounts().get_balance(account.id)?;
    Ok(serde_json::to_string(&json!({
        "found": true,
        "id": i64::from(account.id),
        "number": account.number,
        "name": account.name,
        "type": account.account_type.to_string(),
        "is_placeholder": account.is_placeholder,
        "is_active": account.is_active,
        "balance": format!("${balance}"),
    }))?)
}

fn handle_get_account_children(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    use crate::types::AccountId;
    let account_id = require_i64(params, "account_id").map(AccountId::from)?;
    let children = db.accounts().get_children(account_id)?;
    let rows: Vec<Value> = children
        .iter()
        .map(|a| {
            let balance = db
                .accounts()
                .get_balance(a.id)
                .unwrap_or(crate::types::Money(0));
            json!({
                "id": i64::from(a.id),
                "number": a.number,
                "name": a.name,
                "type": a.account_type.to_string(),
                "is_placeholder": a.is_placeholder,
                "balance": format!("${balance}"),
            })
        })
        .collect();
    Ok(serde_json::to_string(&json!({ "children": rows }))?)
}

fn handle_search_accounts(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    let query = require_string(params, "query")?;
    let accounts = db.accounts().search(query)?;
    let rows: Vec<Value> = accounts
        .iter()
        .map(|a| {
            let balance = db
                .accounts()
                .get_balance(a.id)
                .unwrap_or(crate::types::Money(0));
            json!({
                "id": i64::from(a.id),
                "number": a.number,
                "name": a.name,
                "type": a.account_type.to_string(),
                "is_placeholder": a.is_placeholder,
                "balance": format!("${balance}"),
            })
        })
        .collect();
    Ok(serde_json::to_string(
        &json!({ "accounts": rows, "count": rows.len() }),
    )?)
}

fn handle_get_gl_transactions(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    use crate::types::AccountId;
    let account_id = require_i64(params, "account_id").map(AccountId::from)?;
    let from = params["start_date"].as_str().and_then(|s| s.parse().ok());
    let to = params["end_date"].as_str().and_then(|s| s.parse().ok());
    let date_range = if from.is_some() || to.is_some() {
        Some(DateRange { from, to })
    } else {
        None
    };
    let rows = db
        .journals()
        .list_lines_for_account(account_id, date_range)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "je_number": r.je_number,
                "date": r.entry_date.to_string(),
                "memo": r.memo,
                "debit": format!("${}", r.debit),
                "credit": format!("${}", r.credit),
                "running_balance": format!("${}", r.running_balance),
            })
        })
        .collect();
    Ok(serde_json::to_string(&json!({
        "transactions": items,
        "count": items.len(),
    }))?)
}

fn handle_get_journal_entry(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    use crate::db::journal_repo::JournalFilter;

    // Accept integer (2), string ("JE-0002", "0002", "2"), strip prefix and leading zeros.
    let je_number_str = if let Some(n) = params["je_number"].as_i64() {
        format!("JE-{n:04}")
    } else if let Some(s) = params["je_number"].as_str() {
        let digits = s
            .strip_prefix("JE-")
            .or_else(|| s.strip_prefix("je-"))
            .unwrap_or(s);
        let n: u32 = digits
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid je_number '{s}'"))?;
        format!("JE-{n:04}")
    } else {
        return Err(anyhow::anyhow!(
            "Required parameter 'je_number' missing or not an integer/string"
        ));
    };

    // Find the JE by scanning (no direct get_by_number on JournalRepo).
    let all = db.journals().list(&JournalFilter::default())?;
    let je = all
        .into_iter()
        .find(|j| j.je_number == je_number_str)
        .ok_or_else(|| anyhow::anyhow!("Journal entry {je_number_str} not found"))?;
    let (je, lines) = db.journals().get_with_lines(je.id)?;
    let line_items: Vec<Value> = lines
        .iter()
        .map(|l| {
            let account = db
                .accounts()
                .get_by_id(l.account_id)
                .ok()
                .map(|a| format!("{} - {}", a.number, a.name))
                .unwrap_or_else(|| format!("account #{}", i64::from(l.account_id)));
            json!({
                "account": account,
                "debit": format!("${}", l.debit_amount),
                "credit": format!("${}", l.credit_amount),
                "memo": l.line_memo,
            })
        })
        .collect();
    Ok(serde_json::to_string(&json!({
        "je_number": je.je_number,
        "date": je.entry_date.to_string(),
        "memo": je.memo,
        "status": je.status.to_string(),
        "lines": line_items,
    }))?)
}

fn handle_get_open_ar_items(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    use crate::db::ar_repo::ArFilter;
    let status = params["status"]
        .as_str()
        .map(|s| {
            s.parse::<ArApStatus>()
                .map_err(|e| anyhow::anyhow!("Invalid AR status '{s}': {e}"))
        })
        .transpose()?;
    let items = db.ar().list(&ArFilter { status })?;
    let rows: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "id": i64::from(item.id),
                "customer": item.customer_name,
                "description": item.description,
                "amount": format!("${}", item.amount),
                "due_date": item.due_date.to_string(),
                "status": item.status.to_string(),
            })
        })
        .collect();
    Ok(serde_json::to_string(
        &json!({ "ar_items": rows, "count": rows.len() }),
    )?)
}

fn handle_get_open_ap_items(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    use crate::db::ap_repo::ApFilter;
    let status = params["status"]
        .as_str()
        .map(|s| {
            s.parse::<ArApStatus>()
                .map_err(|e| anyhow::anyhow!("Invalid AP status '{s}': {e}"))
        })
        .transpose()?;
    let items = db.ap().list(&ApFilter { status })?;
    let rows: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "id": i64::from(item.id),
                "vendor": item.vendor_name,
                "description": item.description,
                "amount": format!("${}", item.amount),
                "due_date": item.due_date.to_string(),
                "status": item.status.to_string(),
            })
        })
        .collect();
    Ok(serde_json::to_string(
        &json!({ "ap_items": rows, "count": rows.len() }),
    )?)
}

fn handle_get_envelope_balances(db: &EntityDb) -> anyhow::Result<String> {
    let allocations = db.envelopes().get_all_allocations()?;
    let rows: Vec<Value> = allocations
        .iter()
        .map(|alloc| {
            let account = db
                .accounts()
                .get_by_id(alloc.account_id)
                .ok()
                .map(|a| format!("{} - {}", a.number, a.name))
                .unwrap_or_else(|| format!("account #{}", i64::from(alloc.account_id)));
            let balance = db
                .envelopes()
                .get_balance(alloc.account_id)
                .unwrap_or(crate::types::Money(0));
            json!({
                "account": account,
                "allocation_pct": format!("{}", alloc.percentage),
                "balance": format!("${balance}"),
            })
        })
        .collect();
    Ok(serde_json::to_string(&json!({ "envelopes": rows }))?)
}

fn handle_get_trial_balance(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    let balances = if let Some(as_of) = params["as_of_date"].as_str() {
        let date = as_of
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid as_of_date '{as_of}': {e}"))?;
        db.accounts().get_all_balances_as_of(date)?
    } else {
        db.accounts().get_all_balances()?
    };

    let accounts = db.accounts().list_active()?;
    let rows: Vec<Value> = accounts
        .iter()
        .filter_map(|a| {
            let balance = *balances.get(&a.id)?;
            // Only include accounts with non-zero balances in the trial balance.
            if balance == crate::types::Money(0) {
                return None;
            }
            // Debit-normal: Asset, Expense → positive balance = debit
            // Credit-normal: Liability, Equity, Revenue → positive balance = credit
            use crate::types::AccountType;
            let (debit, credit) = match a.account_type {
                AccountType::Asset | AccountType::Expense => {
                    if balance >= crate::types::Money(0) {
                        (format!("${balance}"), "$0.00".to_string())
                    } else {
                        let abs = crate::types::Money(-balance.0);
                        ("$0.00".to_string(), format!("${abs}"))
                    }
                }
                _ => {
                    if balance >= crate::types::Money(0) {
                        ("$0.00".to_string(), format!("${balance}"))
                    } else {
                        let abs = crate::types::Money(-balance.0);
                        (format!("${abs}"), "$0.00".to_string())
                    }
                }
            };
            Some(json!({
                "number": a.number,
                "name": a.name,
                "type": a.account_type.to_string(),
                "debit": debit,
                "credit": credit,
            }))
        })
        .collect();

    Ok(serde_json::to_string(&json!({
        "trial_balance": rows,
        "count": rows.len(),
    }))?)
}

fn handle_get_audit_log(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    let action_type = params["action"]
        .as_str()
        .map(|s| {
            s.parse::<AuditAction>()
                .map_err(|e| anyhow::anyhow!("Invalid action type '{s}': {e}"))
        })
        .transpose()?;
    let start_date = params["start_date"].as_str().and_then(|s| s.parse().ok());
    let end_date = params["end_date"].as_str().and_then(|s| s.parse().ok());
    let limit = params["limit"].as_u64().map(|n| n as usize).unwrap_or(50);

    let entries = db
        .audit()
        .get_ai_entries(start_date, end_date, Some(limit))
        .or_else(|_| {
            // Fallback: filter manually from all entries.
            use crate::db::audit_repo::AuditFilter;
            let from_str = params["start_date"]
                .as_str()
                .map(|s| format!("{s} 00:00:00"));
            let to_str = params["end_date"].as_str().map(|s| format!("{s} 23:59:59"));
            db.audit().list(&AuditFilter {
                from: from_str,
                to: to_str,
                action_type,
            })
        })?;

    let filtered: Vec<Value> = entries
        .iter()
        .filter(|e| action_type.map(|a| e.action_type == a).unwrap_or(true))
        .take(limit)
        .map(|e| {
            json!({
                "id": i64::from(e.id),
                "action": e.action_type.to_string(),
                "entity": e.entity_name,
                "description": e.description,
                "created_at": e.created_at,
            })
        })
        .collect();

    Ok(serde_json::to_string(&json!({
        "entries": filtered,
        "count": filtered.len(),
    }))?)
}

fn handle_get_tax_tag(params: &Value, db: &EntityDb) -> anyhow::Result<String> {
    use crate::db::journal_repo::JournalFilter;

    let je_number_input = require_string(params, "je_number")?;

    // Normalise to "JE-XXXX" format (accept "JE-4", "4", "0004" etc.).
    let digits = je_number_input
        .strip_prefix("JE-")
        .or_else(|| je_number_input.strip_prefix("je-"))
        .unwrap_or(je_number_input);
    let n: u32 = digits
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid je_number '{je_number_input}'"))?;
    let je_number_str = format!("JE-{n:04}");

    let all = db.journals().list(&JournalFilter::default())?;
    let je = all
        .into_iter()
        .find(|j| j.je_number == je_number_str)
        .ok_or_else(|| anyhow::anyhow!("Journal entry {je_number_str} not found"))?;

    match db.tax_tags().get_for_je(je.id)? {
        None => Ok(serde_json::to_string(&serde_json::json!({
            "je_number": je_number_str,
            "status": "unreviewed",
            "message": "No tax tag — this JE has not been reviewed yet.",
        }))?),
        Some(tag) => Ok(serde_json::to_string(&serde_json::json!({
            "je_number": je_number_str,
            "status": tag.status.to_string(),
            "form_tag": tag.form_tag.as_ref().map(|f| f.to_string()),
            "form_display": tag.form_tag.as_ref().map(|f| f.display_name()),
            "ai_suggested_form": tag.ai_suggested_form.as_ref().map(|f| f.to_string()),
            "ai_suggested_form_display": tag.ai_suggested_form.as_ref().map(|f| f.display_name()),
            "reason": tag.reason,
            "reviewed_at": tag.reviewed_at,
        }))?),
    }
}

// ── Parameter Helpers ─────────────────────────────────────────────────────────

fn require_string<'a>(params: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    params[key]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Required parameter '{key}' missing or not a string"))
}

fn require_i64(params: &Value, key: &str) -> anyhow::Result<i64> {
    params[key]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("Required parameter '{key}' missing or not an integer"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::EntityDb;

    // ── tool_definitions (also tested in earlier unit tests) ───────────────

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

        let no_required: &[&str] = &[
            "get_open_ar_items",
            "get_open_ap_items",
            "get_envelope_balances",
            "get_trial_balance",
            "get_audit_log",
        ];
        for name in no_required {
            let schema = &defs_map[*name].input_schema;
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

    // ── fulfill_tool_call ──────────────────────────────────────────────────

    fn make_tool_call(name: &str, input: Value) -> ToolCall {
        ToolCall {
            id: "tc_test".to_string(),
            name: name.to_string(),
            input,
        }
    }

    #[test]
    fn unknown_tool_name_returns_error_string_not_panic() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("does_not_exist", json!({}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        assert!(result.contains("unknown tool"), "Got: {result}");
    }

    #[test]
    fn get_account_returns_json_for_known_account() {
        let db = EntityDb::open_in_memory().unwrap();
        // Seeded DB has accounts — search for "Cash" which is a common default.
        let tc = make_tool_call("get_account", json!({"query": "Cash"}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).expect("not valid JSON");
        assert!(json.is_object(), "Result is not a JSON object");
    }

    #[test]
    fn get_account_not_found_returns_found_false() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_account", json!({"query": "ZZZNEVEREXISTS9999"}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["found"], false);
    }

    #[test]
    fn get_account_missing_required_param_returns_error() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_account", json!({}));
        // The error is returned as Err(AiError::ParseError(...)).
        let result = fulfill_tool_call(&tc, &db);
        assert!(result.is_err(), "Expected error for missing param");
    }

    #[test]
    fn search_accounts_returns_json_array() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("search_accounts", json!({"query": ""}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json["accounts"].is_array());
        assert!(json["count"].is_number());
    }

    #[test]
    fn get_account_children_returns_json() {
        let db = EntityDb::open_in_memory().unwrap();
        // Look up a placeholder account first.
        let accounts = db.accounts().list_all().unwrap();
        let placeholder = accounts.iter().find(|a| a.is_placeholder);
        if let Some(acc) = placeholder {
            let tc = make_tool_call(
                "get_account_children",
                json!({"account_id": i64::from(acc.id)}),
            );
            let result = fulfill_tool_call(&tc, &db).unwrap();
            let json: Value = serde_json::from_str(&result).unwrap();
            assert!(json["children"].is_array());
        }
        // If no placeholder exists in seed data, just verify the call doesn't panic.
    }

    #[test]
    fn get_gl_transactions_returns_json() {
        let db = EntityDb::open_in_memory().unwrap();
        let accounts = db.accounts().list_active().unwrap();
        let first = &accounts[0];
        let tc = make_tool_call(
            "get_gl_transactions",
            json!({"account_id": i64::from(first.id)}),
        );
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json["transactions"].is_array());
        assert!(json["count"].is_number());
    }

    #[test]
    fn get_open_ar_items_returns_json() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_open_ar_items", json!({}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json["ar_items"].is_array());
    }

    #[test]
    fn get_open_ap_items_returns_json() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_open_ap_items", json!({}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json["ap_items"].is_array());
    }

    #[test]
    fn get_envelope_balances_returns_json() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_envelope_balances", json!({}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json["envelopes"].is_array());
    }

    #[test]
    fn get_trial_balance_returns_json() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_trial_balance", json!({}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json["trial_balance"].is_array());
        assert!(json["count"].is_number());
    }

    #[test]
    fn get_audit_log_returns_json() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_audit_log", json!({}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json["entries"].is_array());
        assert!(json["count"].is_number());
    }

    #[test]
    fn get_open_ar_items_invalid_status_returns_error() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_open_ar_items", json!({"status": "Invalid"}));
        let result = fulfill_tool_call(&tc, &db);
        assert!(result.is_err(), "Expected error for invalid status");
    }

    #[test]
    fn get_journal_entry_integer_not_found_returns_error() {
        let db = EntityDb::open_in_memory().unwrap();
        // No JEs in seed data — any lookup should return not-found (not a parse error).
        let tc = make_tool_call("get_journal_entry", json!({"je_number": 1}));
        let result = fulfill_tool_call(&tc, &db);
        assert!(result.is_err(), "Expected error for missing JE");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "Expected 'not found' in: {err}");
    }

    #[test]
    fn get_journal_entry_string_je_prefix_not_found_returns_error() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_journal_entry", json!({"je_number": "JE-0001"}));
        let result = fulfill_tool_call(&tc, &db);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "Expected 'not found' in: {err}");
    }

    #[test]
    fn get_journal_entry_string_digits_not_found_returns_error() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_journal_entry", json!({"je_number": "0001"}));
        let result = fulfill_tool_call(&tc, &db);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "Expected 'not found' in: {err}");
    }

    #[test]
    fn get_journal_entry_missing_param_returns_error() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_journal_entry", json!({}));
        let result = fulfill_tool_call(&tc, &db);
        assert!(result.is_err(), "Expected error for missing je_number");
    }

    #[test]
    fn get_journal_entry_invalid_string_returns_error() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("get_journal_entry", json!({"je_number": "not-a-number"}));
        let result = fulfill_tool_call(&tc, &db);
        assert!(
            result.is_err(),
            "Expected error for invalid je_number string"
        );
    }

    #[test]
    fn money_values_are_human_readable_in_output() {
        let db = EntityDb::open_in_memory().unwrap();
        let tc = make_tool_call("search_accounts", json!({"query": ""}));
        let result = fulfill_tool_call(&tc, &db).unwrap();
        let json: Value = serde_json::from_str(&result).unwrap();
        // All balance values should be formatted as "$X.XX" strings.
        let accounts = json["accounts"].as_array().unwrap();
        assert!(!accounts.is_empty());
        for acc in accounts {
            let balance = acc["balance"].as_str().unwrap();
            assert!(
                balance.starts_with('$') || balance.starts_with("-$"),
                "Balance not formatted as $X.XX: {balance}"
            );
        }
    }
}
