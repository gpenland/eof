//! Live change feed. Every mutation (from the UI or from an AI over MCP) is broadcast so open
//! browsers refresh instantly and can say *who* changed what.

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::State,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use futures::Stream;
use serde::Serialize;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

use crate::{auth::Owner, state::AppState};

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    /// Which area changed: tasks | countdowns | news | settings
    pub area: &'static str,
    /// "you" for the UI, otherwise the connected app's name (e.g. "Claude").
    pub actor: String,
    /// Human sentence, e.g. `added “Quiz 4”`.
    pub message: String,
}

/// Who is performing an action. Recorded as provenance on created rows.
#[derive(Clone, Debug)]
pub struct Actor(pub String);

impl Actor {
    pub fn you() -> Self {
        Actor("you".into())
    }
    pub fn name(&self) -> &str {
        &self.0
    }
}

pub fn emit(state: &AppState, area: &'static str, actor: &Actor, message: impl Into<String>) {
    let _ = state.events.send(Event { area, actor: actor.0.clone(), message: message.into() });
}

pub async fn stream(
    State(state): State<AppState>,
    _owner: Owner,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = state.events.subscribe();
    let s = BroadcastStream::new(rx).filter_map(|ev| {
        let ev = ev.ok()?;
        Some(Ok(SseEvent::default().event("change").json_data(ev).ok()?))
    });
    Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}
