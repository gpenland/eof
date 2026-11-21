//! MCP server (Streamable HTTP transport, JSON responses).
//!
//! Every capability of the web UI is a tool here, implemented by the same `actions` the UI uses.
//! Changes made by an AI are broadcast live to open browsers and stamped with the AI's name.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    actions,
    domain::{self, countdowns, milestones, news, tasks},
    error::{AppError, AppResult},
    events::Actor,
    oauth::Machine,
    state::AppState,
};

const PROTOCOLS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "EOF is the user's personal productivity app: a todo list, \
a countdown of upcoming dates, and a daily news edition. Start with get_overview to see what's due. \
Dates are YYYY-MM-DD in the user's timezone (\"today\", \"tomorrow\" and weekday names also work). \
Tags are lowercase words without '#'. Countdowns can carry a T-minus plan of tasks (milestones) that move with the date. To deliver the daily news, call publish_issue with plain Markdown: \
'## Section' headings, short paragraphs and bullet lists, links inline; no images.";

pub async fn get_not_allowed() -> impl IntoResponse {
    (StatusCode::METHOD_NOT_ALLOWED, [("allow", "POST")], "This MCP server does not stream; POST JSON-RPC instead.")
}

pub async fn handle(State(st): State<AppState>, m: Machine, _h: HeaderMap, Json(body): Json<Value>) -> Response {
    match body {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for msg in batch {
                if let Some(r) = handle_one(&st, &m.actor, msg).await {
                    out.push(r);
                }
            }
            if out.is_empty() { StatusCode::ACCEPTED.into_response() } else { Json(Value::Array(out)).into_response() }
        }
        msg => match handle_one(&st, &m.actor, msg).await {
            Some(r) => Json(r).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

async fn handle_one(st: &AppState, actor: &Actor, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    // Notifications (no id) and client responses get no reply.
    let id = id?;
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOLS[0]);
            let version = PROTOCOLS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOLS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "eof", "title": "EOF", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call(st, actor, &name, &args).await {
                // Compact text only: no tool declares an outputSchema, so a structuredContent copy
                // would just double every result.
                Ok(v) => json!({
                    "content": [{ "type": "text", "text": tidy(v).to_string() }],
                    "isError": false,
                }),
                Err(e) => json!({
                    "content": [{ "type": "text", "text": format!("Error: {}", e.public_message()) }],
                    "isError": true,
                }),
            })
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

// ── Argument helpers ───────────────────────────────────────────────────────

fn s(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(Value::as_str).map(String::from)
}

fn req_s(a: &Value, k: &str) -> AppResult<String> {
    s(a, k).ok_or_else(|| AppError::bad(format!("`{k}` is required")))
}

fn uuid(a: &Value, k: &str) -> AppResult<Uuid> {
    req_s(a, k)?.parse().map_err(|_| AppError::bad(format!("`{k}` must be an id from a list_* tool")))
}

fn opt_uuid(a: &Value, k: &str) -> AppResult<Option<Uuid>> {
    s(a, k).filter(|v| !v.is_empty()).map(|v| v.parse().map_err(|_| AppError::bad(format!("`{k}` must be an id")))).transpose()
}

fn tags(a: &Value) -> Option<Vec<String>> {
    a.get("tags").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_str).map(String::from).collect())
}

async fn date(st: &AppState, a: &Value, k: &str) -> AppResult<Option<NaiveDate>> {
    match s(a, k) {
        Some(v) if !v.is_empty() => Ok(Some(domain::parse_date(&v, domain::today(&st.db).await)?)),
        _ => Ok(None),
    }
}

/// For patches: absent → None, null → Some(None), value → Some(Some(v)).
fn tri(a: &Value, k: &str) -> Option<Option<String>> {
    match a.get(k) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(v.as_str().map(String::from).filter(|s| !s.is_empty())),
    }
}

async fn edition(st: &AppState, a: &Value) -> AppResult<NaiveDate> {
    date(st, a, "edition").await?.ok_or_else(|| AppError::bad("`edition` (YYYY-MM-DD) is required"))
}

// ── Output shaping ─────────────────────────────────────────────────────────

/// Strip noise from tool output, recursively: null/empty fields, ordering and bookkeeping
/// columns, and a countdown's UTC `at` instant. The model works from the local `date`/`time`
/// and `days_until`; an all-day `at` is midnight UTC and reads as the previous evening locally.
fn tidy(v: Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.into_iter().map(tidy).collect()),
        Value::Object(mut o) => {
            o.remove("position");
            o.remove("updated_at");
            if o.contains_key("all_day") {
                o.remove("at");
            }
            Value::Object(
                o.into_iter()
                    .filter(|(_, v)| !matches!(v, Value::Null) && v != "" && v.as_array().is_none_or(|a| !a.is_empty()))
                    .map(|(k, v)| (k, tidy(v)))
                    .collect(),
            )
        }
        v => v,
    }
}

/// What a write needs to confirm: the id and the fields that identify and schedule the item.
fn brief(v: impl serde::Serialize) -> Value {
    const KEEP: [&str; 11] =
        ["id", "name", "title", "due", "repeat", "starred", "tags", "completed_at", "date", "time", "days_until"];
    match json!(v) {
        Value::Object(o) => Value::Object(o.into_iter().filter(|(k, _)| KEEP.contains(&k.as_str())).collect()),
        v => v,
    }
}

// ── Dispatch ───────────────────────────────────────────────────────────────

pub(crate) async fn call(st: &AppState, actor: &Actor, name: &str, a: &Value) -> AppResult<Value> {
    let db = &st.db;
    Ok(match name {
        "get_overview" => actions::overview(st).await?,

        "list_tasks" => {
            let q = tasks::TaskQuery {
                list_id: opt_uuid(a, "list_id")?,
                status: s(a, "status"),
                tag: s(a, "tag"),
                search: s(a, "search"),
                due_before: date(st, a, "due_before").await?,
                due_after: date(st, a, "due_after").await?,
                limit: a.get("limit").and_then(Value::as_i64),
            };
            json!({ "tasks": tasks::query(db, &q).await? })
        }
        "get_task" => json!(tasks::get(db, uuid(a, "id")?).await?),
        "create_task" => {
            let input = tasks::NewTask {
                list_id: opt_uuid(a, "list_id")?,
                title: req_s(a, "title")?,
                notes: s(a, "notes"),
                tags: tags(a),
                due: date(st, a, "due").await?,
                repeat: s(a, "repeat").filter(|r| !r.is_empty()),
                starred: a.get("starred").and_then(Value::as_bool),
            };
            brief(actions::create_task(st, actor, input).await?)
        }
        "quick_add" => brief(actions::quick_add(st, actor, &req_s(a, "text")?, opt_uuid(a, "list_id")?).await?),
        "update_task" => {
            let today = domain::today(db).await;
            let due = match tri(a, "due") {
                None => None,
                Some(None) => Some(None),
                Some(Some(d)) => Some(Some(domain::parse_date(&d, today)?)),
            };
            let p = tasks::TaskPatch {
                title: s(a, "title"),
                notes: s(a, "notes"),
                tags: tags(a),
                due,
                repeat: tri(a, "repeat"),
                starred: a.get("starred").and_then(Value::as_bool),
                list_id: opt_uuid(a, "list_id")?,
            };
            brief(actions::update_task(st, actor, uuid(a, "id")?, p).await?)
        }
        "complete_task" => {
            let mut r = actions::complete_task(st, actor, uuid(a, "id")?).await?;
            r["task"] = brief(r["task"].take());
            r
        }
        "reopen_task" => brief(actions::uncomplete_task(st, actor, uuid(a, "id")?).await?),
        "delete_task" => json!({ "deleted": brief(actions::delete_task(st, actor, uuid(a, "id")?).await?) }),
        "move_task" => brief(actions::move_task(st, actor, uuid(a, "id")?, opt_uuid(a, "after_id")?).await?),
        "clear_completed_tasks" => json!({ "cleared": actions::clear_completed(st, actor, uuid(a, "list_id")?).await? }),
        "list_task_lists" => json!({ "lists": tasks::lists(db).await? }),
        "create_task_list" => brief(actions::create_list(st, actor, &req_s(a, "name")?).await?),
        "rename_task_list" => {
            actions::rename_list(st, actor, uuid(a, "id")?, &req_s(a, "name")?).await?;
            json!({ "ok": true })
        }
        "delete_task_list" => {
            actions::delete_list(st, actor, uuid(a, "id")?).await?;
            json!({ "ok": true })
        }
        "list_tags" => {
            let t = tasks::all_tags(db).await?;
            json!({ "tags": t.into_iter().map(|(tag, n)| json!({ "tag": tag, "open_tasks": n })).collect::<Vec<_>>() })
        }

        "list_countdowns" => {
            let tz = domain::timezone(db).await;
            let today = domain::today(db).await;
            let past = a.get("include_past").and_then(Value::as_bool).unwrap_or(false);
            let mut v: Vec<_> = countdowns::list(db, past).await?.into_iter().map(|c| countdowns::view(c, tz, today)).collect();
            milestones::attach(db, &mut v).await?;
            json!({ "countdowns": v })
        }
        "create_countdown" => {
            let input = countdowns::NewCountdown {
                title: req_s(a, "title")?,
                date: date(st, a, "date").await?.ok_or_else(|| AppError::bad("`date` is required"))?,
                time: s(a, "time").filter(|t| !t.is_empty()),
                note: s(a, "note"),
                milestones: match a.get("milestones") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(v) => serde_json::from_value(v.clone())
                        .map_err(|_| AppError::bad("`milestones` must be a list of {title, days_before}"))?,
                },
            };
            let v = actions::create_countdown(st, actor, input).await?;
            let plan: Vec<Value> = v.milestones.iter().map(|m| json!({ "id": m.id, "title": m.title, "due": m.due })).collect();
            let mut out = brief(&v);
            if !plan.is_empty() {
                out["milestones"] = json!(plan);
            }
            out
        }
        "update_countdown" => {
            let p = countdowns::CountdownPatch {
                title: s(a, "title"),
                date: date(st, a, "date").await?,
                time: tri(a, "time"),
                note: s(a, "note"),
            };
            brief(actions::update_countdown(st, actor, uuid(a, "id")?, p).await?)
        }
        "add_milestone" => {
            let days_before = a.get("days_before").and_then(Value::as_i64).ok_or_else(|| AppError::bad("`days_before` is required"))?;
            let m = milestones::NewMilestone { title: req_s(a, "title")?, days_before: days_before.clamp(-10_000, 10_000) as i32 };
            brief(actions::add_milestone(st, actor, uuid(a, "countdown_id")?, m).await?)
        }
        "delete_countdown" => {
            actions::delete_countdown(st, actor, uuid(a, "id")?).await?;
            json!({ "ok": true })
        }

        "publish_issue" => {
            let i = actions::publish_issue(st, actor, date(st, a, "edition").await?, s(a, "title"), &req_s(a, "markdown")?).await?;
            json!({
                "ok": true,
                "edition": i.edition,
                "title": i.title,
                "url": format!("{}/news/{}", st.cfg.origin(), i.edition),
                "sections": news::sections(&i.markdown),
            })
        }
        "list_issues" => {
            let limit = a.get("limit").and_then(Value::as_i64).unwrap_or(14);
            json!({ "issues": news::list(db, limit).await? })
        }
        "get_issue" => {
            let which = s(a, "edition").unwrap_or_else(|| "latest".into());
            let which = if which == "latest" { which } else { domain::parse_date(&which, domain::today(db).await)?.to_string() };
            let r = news::get(db, &which).await?;
            json!({
                "edition": r.issue.edition, "title": r.issue.title, "markdown": r.issue.markdown,
                "read": r.issue.read_at.is_some(), "minutes": r.minutes, "sections": r.sections,
            })
        }
        "set_issue_read" => {
            let read = a.get("read").and_then(Value::as_bool).unwrap_or(true);
            actions::set_issue_read(st, actor, edition(st, a).await?, read).await?;
            json!({ "ok": true })
        }
        "delete_issue" => {
            actions::delete_issue(st, actor, edition(st, a).await?).await?;
            json!({ "ok": true })
        }
        "email_issue" => {
            let i = actions::email_issue(st, actor, edition(st, a).await?).await?;
            json!({ "ok": true, "edition": i.edition, "emailed_at": i.emailed_at })
        }

        "set_timezone" => {
            domain::set_timezone(db, &req_s(a, "timezone")?).await?;
            json!({ "ok": true })
        }
        other => return Err(AppError::bad(format!("unknown tool {other}"))),
    })
}

// ── Tool catalogue ─────────────────────────────────────────────────────────

fn tool(name: &str, title: &str, desc: &str, props: Value, required: &[&str], read_only: bool, destructive: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": { "type": "object", "properties": props, "required": required, "additionalProperties": false },
        "annotations": { "title": title, "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": read_only, "openWorldHint": false },
    })
}

pub(crate) fn tool_list() -> Vec<Value> {
    let id = json!({ "type": "string", "description": "Task id (uuid) from list_tasks" });
    let date = |d: &str| json!({ "type": "string", "description": d });
    let repeat = json!({ "type": ["string", "null"], "enum": ["daily", "weekdays", "weekly", "monthly", "yearly", null],
                          "description": "Repeat rule. Completing a repeating task logs it and moves the due date forward." });
    let tags = json!({ "type": "array", "items": { "type": "string" }, "description": "Lowercase tags, e.g. [\"cs2110\", \"exam\"]" });
    vec![
        tool("get_overview", "Today at a glance", "Overdue tasks, tasks due today and this week, the next countdowns, and whether today's news issue is in. Call this first.", json!({}), &[], true, false),

        tool("list_tasks", "List tasks", "Search and filter tasks. Defaults to open tasks in all lists, sorted by due date then custom order.", json!({
            "list_id": { "type": "string", "description": "Restrict to one list (see list_task_lists)" },
            "status": { "type": "string", "enum": ["open", "completed", "all"] },
            "tag": { "type": "string" },
            "search": { "type": "string", "description": "Substring match on title and notes" },
            "due_before": date("Only tasks due on or before this date"),
            "due_after": date("Only tasks due on or after this date"),
            "limit": { "type": "integer", "minimum": 1, "maximum": 2000 },
        }), &[], true, false),
        tool("get_task", "Get task", "Fetch one task by id.", json!({ "id": id }), &["id"], true, false),
        tool("create_task", "Add task", "Add a task (goes to the top of the list).", json!({
            "title": { "type": "string" },
            "notes": { "type": "string", "description": "Optional details" },
            "due": date("Due date: YYYY-MM-DD, 'today', 'tomorrow', or a weekday name"),
            "repeat": repeat,
            "tags": tags,
            "starred": { "type": "boolean" },
            "list_id": { "type": "string", "description": "Defaults to the first list" },
        }), &["title"], false, false),
        tool("quick_add", "Quick add task", "Add a task from one natural sentence, parsed exactly like the user's own composer: dates ('fri', 'tomorrow', 'next week', 'in 3 days', 'dec 5', '12/5'), repeats ('every monday', 'daily'), '#tags' and a trailing '!' to star. E.g. \"Quiz 4 fri #cs2110 !\". Prefer create_task when you already have structured fields.", json!({
            "text": { "type": "string" },
            "list_id": { "type": "string", "description": "Defaults to the first list" },
        }), &["text"], false, false),
        tool("update_task", "Edit task", "Change any fields of a task. Pass null for due or repeat to clear them.", json!({
            "id": id,
            "title": { "type": "string" },
            "notes": { "type": "string" },
            "due": { "type": ["string", "null"], "description": "YYYY-MM-DD, 'today', 'tomorrow', weekday, or null to clear" },
            "repeat": repeat,
            "tags": tags,
            "starred": { "type": "boolean" },
            "list_id": { "type": "string", "description": "Move to another list" },
        }), &["id"], false, false),
        tool("complete_task", "Complete task", "Mark a task done. Repeating tasks roll forward to their next date instead.", json!({ "id": id }), &["id"], false, false),
        tool("reopen_task", "Reopen task", "Mark a completed task as not done.", json!({ "id": id }), &["id"], false, false),
        tool("delete_task", "Delete task", "Permanently delete a task.", json!({ "id": id }), &["id"], false, true),
        tool("move_task", "Reorder task", "Custom ordering: place a task directly after another open task in the same list, or at the top if after_id is omitted.", json!({
            "id": id, "after_id": { "type": "string", "description": "Task to place it after; omit for top" },
        }), &["id"], false, false),
        tool("clear_completed_tasks", "Clear completed", "Delete all completed tasks in a list.", json!({ "list_id": { "type": "string" } }), &["list_id"], false, true),
        tool("list_task_lists", "List task lists", "All task lists with their open-task counts.", json!({}), &[], true, false),
        tool("create_task_list", "Create list", "Create a new task list.", json!({ "name": { "type": "string" } }), &["name"], false, false),
        tool("rename_task_list", "Rename list", "Rename a task list.", json!({ "id": { "type": "string" }, "name": { "type": "string" } }), &["id", "name"], false, false),
        tool("delete_task_list", "Delete list", "Delete a list and every task in it.", json!({ "id": { "type": "string" } }), &["id"], false, true),
        tool("list_tags", "List tags", "Tags in use on open tasks, with counts.", json!({}), &[], true, false),

        tool("list_countdowns", "List countdowns", "Upcoming dates being counted down to, with days_until.", json!({
            "include_past": { "type": "boolean", "description": "Also include events that already happened" },
        }), &[], true, false),
        tool("create_countdown", "Add countdown", "Start counting down to a date.", json!({
            "title": { "type": "string" },
            "date": date("YYYY-MM-DD"),
            "time": { "type": "string", "description": "Optional local time HH:MM (24h). Omit for all-day." },
            "note": { "type": "string" },
            "milestones": { "type": "array", "description": "Optional T-minus plan: tasks due a number of days before the date. They move with the countdown.",
                "items": { "type": "object", "properties": {
                    "title": { "type": "string" },
                    "days_before": { "type": "integer", "description": "0 = on the day; negative = after it" },
                }, "required": ["title", "days_before"], "additionalProperties": false } },
        }), &["title", "date"], false, false),
        tool("update_countdown", "Edit countdown", "Change a countdown. Pass time: null to make it all-day.", json!({
            "id": { "type": "string" }, "title": { "type": "string" }, "date": date("YYYY-MM-DD"),
            "time": { "type": ["string", "null"] }, "note": { "type": "string" },
        }), &["id"], false, false),
        tool("add_milestone", "Plan a countdown step", "Add a step to a countdown's T-minus plan: a task due `days_before` days ahead of it (e.g. book flights 21 days before a trip). It appears in Tasks and moves when the countdown's date changes.", json!({
            "countdown_id": { "type": "string", "description": "From list_countdowns" },
            "title": { "type": "string" },
            "days_before": { "type": "integer", "description": "0 = on the day; negative = after it" },
        }), &["countdown_id", "title", "days_before"], false, false),
        tool("delete_countdown", "Delete countdown", "Remove a countdown.", json!({ "id": { "type": "string" } }), &["id"], false, true),

        tool("publish_issue", "Publish news issue", "Publish the daily news edition. Plain Markdown: optional '# Title' first line, then '## Section' headings, short paragraphs, bullets and inline links; no images. One issue per date — publishing the same date again replaces it. Today's edition is emailed to the user the first time it's published (corrections via republish are not re-sent; use email_issue for that).", json!({
            "markdown": { "type": "string" },
            "title": { "type": "string", "description": "Overrides a leading '# Title' line" },
            "edition": date("Edition date YYYY-MM-DD; defaults to today"),
        }), &["markdown"], false, false),
        tool("list_issues", "List news issues", "Recent editions with title, read state, sections and reading time.", json!({
            "limit": { "type": "integer", "minimum": 1, "maximum": 1000 },
        }), &[], true, false),
        tool("get_issue", "Read news issue", "Get an edition's Markdown. Defaults to the latest.", json!({
            "edition": date("YYYY-MM-DD or 'latest'"),
        }), &[], true, false),
        tool("set_issue_read", "Mark issue read", "Mark an edition read or unread.", json!({
            "edition": date("YYYY-MM-DD"), "read": { "type": "boolean" },
        }), &["edition"], false, false),
        tool("email_issue", "Email news issue", "Email an edition to the user now, even if it was emailed before.", json!({
            "edition": date("YYYY-MM-DD"),
        }), &["edition"], false, false),
        tool("delete_issue", "Delete issue", "Delete an edition.", json!({ "edition": date("YYYY-MM-DD") }), &["edition"], false, true),

        tool("set_timezone", "Set timezone", "Set the user's IANA timezone (e.g. America/New_York). Defines 'today'.", json!({
            "timezone": { "type": "string" },
        }), &["timezone"], false, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_drops_noise_and_countdown_instant() {
        let v = tidy(json!({ "countdowns": [{
            "id": "x", "title": "Trip", "at": "2026-11-22T00:00:00Z", "all_day": true,
            "note": "", "time": null, "date": "2026-11-22", "days_until": 5,
        }], "tasks": [{ "id": "t", "notes": "", "tags": [], "position": 1.0, "updated_at": "z", "starred": false }] }));
        assert_eq!(
            v,
            json!({
                "countdowns": [{ "id": "x", "title": "Trip", "all_day": true, "date": "2026-11-22", "days_until": 5 }],
                "tasks": [{ "id": "t", "starred": false }],
            })
        );
    }

    #[test]
    fn brief_keeps_identity_and_schedule() {
        let v = brief(json!({ "id": "t", "title": "A", "notes": "long", "due": "2026-11-20", "list_id": "l", "created_by": "you" }));
        assert_eq!(v, json!({ "id": "t", "title": "A", "due": "2026-11-20" }));
    }
}
