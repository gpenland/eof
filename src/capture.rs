//! `POST /capture` — add a task from anywhere with one sentence and a token.
//!
//! Built for an iOS Shortcut ("Hey Siri, add to EOF"): send the dictated text with a personal
//! token from Settings and get back a short confirmation to speak or show. The sentence is parsed
//! like the web composer ("pay rent fri #home !"), and the task is stamped with the token's name.
//!
//! Body: JSON `{"text": "...", "list": "Groceries"}`, a form field `text=`, or plain text.
//! Reply: plain text, or JSON when the request sends `Accept: application/json`.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Datelike;
use serde::Deserialize;
use serde_json::json;

use crate::{
    actions,
    domain::{self, tasks},
    error::AppError,
    oauth::Machine,
    state::AppState,
};

#[derive(Default, Deserialize)]
struct Body {
    #[serde(default)]
    text: String,
    /// Optional list name (case-insensitive); defaults to the first list.
    #[serde(default)]
    list: Option<String>,
}

fn read_body(headers: &HeaderMap, raw: &[u8]) -> Body {
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    let text = String::from_utf8_lossy(raw);
    if (ct.starts_with("application/json") || text.trim_start().starts_with('{'))
        && let Ok(b) = serde_json::from_slice::<Body>(raw)
    {
        return b;
    }
    // A real form has a `text=` field. Plenty of clients (curl -d, some Shortcuts body types) label
    // a bare sentence as a form, though; then the body itself is the text.
    if ct.starts_with("application/x-www-form-urlencoded") && url::form_urlencoded::parse(raw).any(|(k, _)| k == "text") {
        let mut b = Body::default();
        for (k, v) in url::form_urlencoded::parse(raw) {
            match k.as_ref() {
                "text" => b.text = v.into_owned(),
                "list" => b.list = Some(v.into_owned()),
                _ => {}
            }
        }
        return b;
    }
    Body { text: text.into_owned(), list: None }
}

pub async fn handle(State(st): State<AppState>, m: Machine, headers: HeaderMap, raw: Bytes) -> Response {
    let wants_json = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("application/json"));
    let reply = |status: StatusCode, msg: String, extra: serde_json::Value| -> Response {
        if wants_json {
            let mut v = json!({ "ok": status.is_success(), "message": msg });
            if let (Some(o), serde_json::Value::Object(e)) = (v.as_object_mut(), extra) {
                o.extend(e);
            }
            (status, axum::Json(v)).into_response()
        } else {
            (status, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], msg).into_response()
        }
    };

    let b = read_body(&headers, &raw);
    let text = b.text.trim();
    if text.is_empty() {
        return reply(StatusCode::BAD_REQUEST, "Nothing to add — send the task as text.".into(), json!({}));
    }
    if text.len() > 1000 {
        return reply(StatusCode::BAD_REQUEST, "That’s too long for a task.".into(), json!({}));
    }
    let list_id = match b.list.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        None => None,
        Some(name) => match tasks::lists(&st.db).await {
            Ok(ls) => match ls.into_iter().find(|l| l.name.eq_ignore_ascii_case(name)) {
                Some(l) => Some(l.id),
                None => return reply(StatusCode::NOT_FOUND, format!("There’s no list called “{name}”."), json!({})),
            },
            Err(e) => return reply(e.status(), e.public_message(), json!({})),
        },
    };
    match actions::quick_add(&st, &m.actor, text, list_id).await {
        Ok(t) => {
            let today = domain::today(&st.db).await;
            let mut msg = format!("Added “{}”", t.title);
            if let Some(d) = t.due {
                let n = (d - today).num_days();
                msg += &match n {
                    0 => " for today".into(),
                    1 => " for tomorrow".into(),
                    2..=6 => format!(" for {}", d.format("%A")),
                    _ if d.year() == today.year() => format!(" for {}", d.format("%b %-d")),
                    _ => format!(" for {}", d.format("%b %-d, %Y")),
                };
            }
            if t.repeat.is_some() {
                msg += ", repeating";
            }
            if !t.tags.is_empty() {
                msg += &format!(" · #{}", t.tags.join(" #"));
            }
            if t.starred {
                msg += " ★";
            }
            reply(StatusCode::OK, msg, json!({ "task": { "id": t.id, "title": t.title, "due": t.due, "tags": t.tags, "starred": t.starred, "repeat": t.repeat } }))
        }
        Err(e @ AppError::BadRequest(_)) => reply(StatusCode::BAD_REQUEST, e.public_message(), json!({})),
        Err(e) => reply(e.status(), e.public_message(), json!({})),
    }
}
