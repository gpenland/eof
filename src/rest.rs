//! `POST /v1/{name}` — a plain HTTP API for scripts, automations and anything that isn't MCP.
//!
//! Every MCP tool is also an endpoint here, with the same name, arguments and behaviour: the
//! request body is the tool's arguments and the response is its result as JSON. So it covers
//! everything the UI can do, and stays in step with MCP without a second list to maintain.
//!
//! Auth: `Authorization: Bearer <token>` (a personal token from Settings, or an OAuth token).
//! Body: a JSON object, a form (`a=1&b=2`; `tags` may be comma-separated), or nothing.
//! Reply: the result as JSON, or `{"error": "..."}` with a 4xx/5xx status.
//! `GET /v1` lists the endpoints with their JSON Schemas.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Map, Value, json};

use crate::{error::AppError, mcp, oauth::Machine, state::AppState};

pub async fn index(State(st): State<AppState>, _m: Machine) -> Json<Value> {
    let base = format!("{}/v1", st.cfg.origin());
    let endpoints: Vec<Value> = mcp::tool_list()
        .into_iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap_or_default();
            json!({
                "name": name,
                "method": "POST",
                "url": format!("{base}/{name}"),
                "title": t["title"],
                "description": t["description"],
                "read_only": t["annotations"]["readOnlyHint"],
                "destructive": t["annotations"]["destructiveHint"],
                "input_schema": t["inputSchema"],
            })
        })
        .collect();
    Json(json!({ "endpoints": endpoints }))
}

pub async fn handle(State(st): State<AppState>, m: Machine, Path(name): Path<String>, headers: HeaderMap, raw: Bytes) -> Response {
    if !mcp::tool_list().iter().any(|t| t["name"] == name.as_str()) {
        let msg = format!("There’s no endpoint called “{name}” — GET /v1 lists them.");
        return (StatusCode::NOT_FOUND, Json(json!({ "error": msg }))).into_response();
    }
    let args = match parse_args(&headers, &raw) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    match mcp::call(&st, &m.actor, &name, &args).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

fn parse_args(headers: &HeaderMap, raw: &[u8]) -> Result<Value, AppError> {
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if raw.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    if ct.starts_with("application/x-www-form-urlencoded") {
        let mut o = Map::new();
        for (k, v) in url::form_urlencoded::parse(raw) {
            let v = if k == "tags" {
                Value::Array(v.split(',').map(str::trim).filter(|t| !t.is_empty()).map(|t| json!(t)).collect())
            } else {
                Value::String(v.into_owned())
            };
            o.insert(k.into_owned(), v);
        }
        return Ok(Value::Object(o));
    }
    match serde_json::from_slice::<Value>(raw) {
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(_) => Err(AppError::bad("The body must be a JSON object of arguments.")),
        Err(e) => Err(AppError::bad(format!("The body isn’t valid JSON: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(ct: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(header::CONTENT_TYPE, ct.parse().unwrap());
        m
    }

    #[test]
    fn parses_json_form_and_empty() {
        assert_eq!(parse_args(&HeaderMap::new(), b"").unwrap(), json!({}));
        assert_eq!(parse_args(&h("application/json"), b" {\"title\":\"x\"} ").unwrap(), json!({ "title": "x" }));
        assert_eq!(
            parse_args(&h("application/x-www-form-urlencoded"), b"title=Pay+rent&tags=home,+bills").unwrap(),
            json!({ "title": "Pay rent", "tags": ["home", "bills"] })
        );
        assert!(parse_args(&h("application/json"), b"[1]").is_err());
        assert!(parse_args(&h("application/json"), b"{nope").is_err());
    }
}
