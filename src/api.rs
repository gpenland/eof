//! JSON API for the web UI. Thin: parse, call `actions`, return.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post, put},
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    actions,
    auth::{self, Owner},
    domain::{self, countdowns, milestones, news, tasks},
    error::{AppError, AppResult},
    events::{self, Actor},
    oauth, push,
    state::AppState,
};

type J = AppResult<Json<Value>>;

fn ok() -> J {
    Ok(Json(json!({ "ok": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/bootstrap", get(bootstrap))
        .route("/overview", get(overview))
        .route("/events", get(events::stream))
        .route("/timezone", put(set_timezone))
        .route("/lists", get(list_lists).post(create_list))
        .route("/lists/{id}", patch(rename_list).delete(delete_list))
        .route("/lists/{id}/clear-completed", post(clear_completed))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/{id}", patch(update_task).delete(delete_task))
        .route("/tasks/{id}/complete", post(complete_task))
        .route("/tasks/{id}/uncomplete", post(uncomplete_task))
        .route("/tasks/{id}/move", post(move_task))
        .route("/tags", get(tags))
        .route("/countdowns", get(list_countdowns).post(create_countdown))
        .route("/countdowns/{id}", patch(update_countdown).delete(delete_countdown))
        .route("/countdowns/{id}/milestones", post(add_milestone))
        .route("/push", get(push_status))
        .route("/push/subscribe", post(push_subscribe))
        .route("/push/unsubscribe", post(push_unsubscribe))
        .route("/push/test", post(push_test))
        .route("/push/digest", put(push_digest))
        .route("/push/{id}", axum::routing::delete(push_remove))
        .route("/issues", get(list_issues).post(publish_issue))
        .route("/issues/{which}", get(get_issue).delete(delete_issue))
        .route("/issues/{edition}/read", put(set_read))
        .route("/issues/{edition}/email", post(email_issue))
        .route("/passkeys", get(auth::list_passkeys))
        .route("/passkeys/{id}", patch(auth::rename_passkey).delete(auth::delete_passkey))
        .route("/sessions", get(auth::list_sessions))
        .route("/sessions/{id}", axum::routing::delete(auth::delete_session))
        .route("/connections", get(oauth::list_grants).post(oauth::create_personal_token))
        .route("/connections/{id}", axum::routing::delete(oauth::delete_grant))
        .route("/oauth/client/{client_id}", get(oauth::client_info))
        .route("/oauth/approve", post(oauth::approve))
        .route("/logout", post(auth::logout))
}

async fn bootstrap(State(st): State<AppState>, _o: Owner) -> J {
    let tz = domain::timezone(&st.db).await;
    Ok(Json(json!({
        "today": domain::today(&st.db).await,
        "timezone": tz.name(),
        "lists": tasks::lists(&st.db).await?,
        "mcp_url": format!("{}/mcp", st.cfg.origin()),
        "origin": st.cfg.origin(),
    })))
}

async fn overview(State(st): State<AppState>, _o: Owner) -> J {
    Ok(Json(actions::overview(&st).await?))
}

#[derive(Deserialize)]
struct Tz {
    timezone: String,
}

async fn set_timezone(State(st): State<AppState>, _o: Owner, Json(b): Json<Tz>) -> J {
    domain::set_timezone(&st.db, &b.timezone).await?;
    ok()
}

// ── Lists ──

#[derive(Deserialize)]
struct Name {
    name: String,
}

async fn list_lists(State(st): State<AppState>, _o: Owner) -> J {
    Ok(Json(json!(tasks::lists(&st.db).await?)))
}

async fn create_list(State(st): State<AppState>, _o: Owner, Json(b): Json<Name>) -> J {
    Ok(Json(json!(actions::create_list(&st, &Actor::you(), &b.name).await?)))
}

async fn rename_list(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>, Json(b): Json<Name>) -> J {
    actions::rename_list(&st, &Actor::you(), id, &b.name).await?;
    ok()
}

async fn delete_list(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>) -> J {
    actions::delete_list(&st, &Actor::you(), id).await?;
    ok()
}

async fn clear_completed(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>) -> J {
    let n = actions::clear_completed(&st, &Actor::you(), id).await?;
    Ok(Json(json!({ "cleared": n })))
}

// ── Tasks ──

async fn list_tasks(State(st): State<AppState>, _o: Owner, Query(q): Query<tasks::TaskQuery>) -> J {
    Ok(Json(json!(tasks::query(&st.db, &q).await?)))
}

async fn create_task(State(st): State<AppState>, _o: Owner, Json(b): Json<tasks::NewTask>) -> J {
    Ok(Json(json!(actions::create_task(&st, &Actor::you(), b).await?)))
}

async fn update_task(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>, Json(b): Json<tasks::TaskPatch>) -> J {
    Ok(Json(json!(actions::update_task(&st, &Actor::you(), id, b).await?)))
}

async fn delete_task(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>) -> J {
    Ok(Json(json!(actions::delete_task(&st, &Actor::you(), id).await?)))
}

async fn complete_task(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>) -> J {
    Ok(Json(actions::complete_task(&st, &Actor::you(), id).await?))
}

async fn uncomplete_task(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>) -> J {
    Ok(Json(json!(actions::uncomplete_task(&st, &Actor::you(), id).await?)))
}

#[derive(Deserialize)]
struct MoveBody {
    after: Option<Uuid>,
}

async fn move_task(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>, Json(b): Json<MoveBody>) -> J {
    Ok(Json(json!(actions::move_task(&st, &Actor::you(), id, b.after).await?)))
}

async fn tags(State(st): State<AppState>, _o: Owner) -> J {
    let t = tasks::all_tags(&st.db).await?;
    Ok(Json(json!(t.into_iter().map(|(tag, n)| json!({ "tag": tag, "count": n })).collect::<Vec<_>>())))
}

// ── Countdowns ──

#[derive(Deserialize)]
struct PastQ {
    #[serde(default)]
    include_past: bool,
}

async fn list_countdowns(State(st): State<AppState>, _o: Owner, Query(q): Query<PastQ>) -> J {
    let tz = domain::timezone(&st.db).await;
    let today = domain::today(&st.db).await;
    let mut v: Vec<_> = countdowns::list(&st.db, q.include_past).await?.into_iter().map(|c| countdowns::view(c, tz, today)).collect();
    milestones::attach(&st.db, &mut v).await?;
    Ok(Json(json!(v)))
}

async fn add_milestone(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>, Json(b): Json<milestones::NewMilestone>) -> J {
    Ok(Json(json!(actions::add_milestone(&st, &Actor::you(), id, b).await?)))
}

async fn create_countdown(State(st): State<AppState>, _o: Owner, Json(b): Json<countdowns::NewCountdown>) -> J {
    Ok(Json(json!(actions::create_countdown(&st, &Actor::you(), b).await?)))
}

async fn update_countdown(
    State(st): State<AppState>,
    _o: Owner,
    Path(id): Path<Uuid>,
    Json(b): Json<countdowns::CountdownPatch>,
) -> J {
    Ok(Json(json!(actions::update_countdown(&st, &Actor::you(), id, b).await?)))
}

async fn delete_countdown(State(st): State<AppState>, _o: Owner, Path(id): Path<Uuid>) -> J {
    actions::delete_countdown(&st, &Actor::you(), id).await?;
    ok()
}

// ── News ──

#[derive(Deserialize)]
struct LimitQ {
    limit: Option<i64>,
}

async fn list_issues(State(st): State<AppState>, _o: Owner, Query(q): Query<LimitQ>) -> J {
    Ok(Json(json!(news::list(&st.db, q.limit.unwrap_or(120)).await?)))
}

async fn get_issue(State(st): State<AppState>, _o: Owner, Path(which): Path<String>) -> J {
    Ok(Json(json!(news::get(&st.db, &which).await?)))
}

#[derive(Deserialize)]
struct PublishBody {
    edition: Option<NaiveDate>,
    title: Option<String>,
    markdown: String,
}

async fn publish_issue(State(st): State<AppState>, _o: Owner, Json(b): Json<PublishBody>) -> J {
    Ok(Json(json!(actions::publish_issue(&st, &Actor::you(), b.edition, b.title, &b.markdown).await?)))
}

#[derive(Deserialize)]
struct ReadBody {
    read: bool,
}

fn edition(s: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| AppError::bad("edition must be YYYY-MM-DD"))
}

async fn set_read(State(st): State<AppState>, _o: Owner, Path(ed): Path<String>, Json(b): Json<ReadBody>) -> J {
    actions::set_issue_read(&st, &Actor::you(), edition(&ed)?, b.read).await?;
    ok()
}

async fn email_issue(State(st): State<AppState>, _o: Owner, Path(ed): Path<String>) -> J {
    Ok(Json(json!(actions::email_issue(&st, &Actor::you(), edition(&ed)?).await?)))
}

async fn delete_issue(State(st): State<AppState>, _o: Owner, Path(ed): Path<String>) -> J {
    actions::delete_issue(&st, &Actor::you(), edition(&ed)?).await?;
    ok()
}

// ── Notifications ──

async fn push_status(State(st): State<AppState>, _o: Owner) -> J {
    let key = push::vapid_key(&st.db).await?;
    Ok(Json(json!({
        "public_key": push::public_key_b64(&key),
        "devices": push::devices(&st.db).await?,
        "digest_hour": push::digest_hour(&st.db).await,
    })))
}

async fn push_subscribe(State(st): State<AppState>, _o: Owner, Json(b): Json<push::Subscription>) -> J {
    let d = push::subscribe(&st.db, b).await?;
    events::emit(&st, "settings", &Actor::you(), format!("turned on notifications for {}", d.label));
    Ok(Json(json!(d)))
}

#[derive(Deserialize)]
struct Endpoint {
    endpoint: String,
}

async fn push_unsubscribe(State(st): State<AppState>, _o: Owner, Json(b): Json<Endpoint>) -> J {
    push::remove(&st.db, &b.endpoint).await?;
    ok()
}

async fn push_remove(State(st): State<AppState>, _o: Owner, Path(id): Path<String>) -> J {
    push::remove(&st.db, &id).await?;
    ok()
}

async fn push_test(State(st): State<AppState>, _o: Owner) -> J {
    let note = match push::digest(&st).await? {
        Some(n) => push::Note { tag: "test".into(), ..n },
        None => push::Note { title: "EOF".into(), body: "Notifications are on. Nothing due — enjoy it.".into(), url: "/tasks".into(), tag: "test".into() },
    };
    let sent = push::send(&st, &note).await?;
    Ok(Json(json!({ "sent": sent })))
}

#[derive(Deserialize)]
struct DigestBody {
    hour: Option<u32>,
}

async fn push_digest(State(st): State<AppState>, _o: Owner, Json(b): Json<DigestBody>) -> J {
    push::set_digest_hour(&st.db, b.hour).await?;
    ok()
}
