//! Countdown milestones: a "T-minus" plan for a date. Each milestone is an ordinary task (it shows
//! up in Tasks, can be completed by you or an AI) that remembers which countdown it belongs to and
//! how many days before it falls. Move the countdown and its open milestones move with it; move a
//! milestone's due date and it keeps the new distance.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::{countdowns, tasks};
use crate::{
    error::{AppError, AppResult},
    events::Actor,
};

#[derive(Debug, Clone, Deserialize)]
pub struct NewMilestone {
    pub title: String,
    /// Days before the countdown the task is due; 0 = the day itself, negative = after.
    pub days_before: i32,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Milestone {
    /// The task's id.
    pub id: Uuid,
    pub title: String,
    pub days_before: i32,
    pub due: Option<NaiveDate>,
    pub done: bool,
}

fn check_offset(n: i32) -> AppResult<i32> {
    if !(-365..=3650).contains(&n) {
        return Err(AppError::bad("days_before must be between -365 and 3650"));
    }
    Ok(n)
}

/// Add a milestone task to the default list, due `days_before` days ahead of the countdown.
pub async fn add(db: &PgPool, tz: Tz, countdown_id: Uuid, m: NewMilestone, actor: &Actor) -> AppResult<tasks::Task> {
    let days_before = check_offset(m.days_before)?;
    let c = countdowns::get(db, countdown_id).await?;
    let due = countdowns::local_date(&c, tz) - Duration::days(days_before as i64);
    let input = tasks::NewTask { title: m.title, due: Some(due), ..Default::default() };
    let t = tasks::create(db, input, actor, due).await?;
    Ok(sqlx::query_as("UPDATE tasks SET countdown_id = $2, days_before = $3 WHERE id = $1 RETURNING *")
        .bind(t.id)
        .bind(countdown_id)
        .bind(days_before)
        .fetch_one(db)
        .await?)
}

/// After a countdown's date changes: move its open milestones along. Returns how many moved.
pub async fn follow(db: &PgPool, tz: Tz, c: &countdowns::Countdown) -> AppResult<u64> {
    let date = countdowns::local_date(c, tz);
    Ok(sqlx::query(
        "UPDATE tasks SET due = $2::date - days_before, updated_at = now()
         WHERE countdown_id = $1 AND completed_at IS NULL AND due IS DISTINCT FROM $2::date - days_before",
    )
    .bind(c.id)
    .bind(date)
    .execute(db)
    .await?
    .rows_affected())
}

/// After a milestone task's due date was edited by hand: keep the new distance, or let go of the
/// countdown if the date was cleared.
pub async fn rebase(db: &PgPool, tz: Tz, t: &tasks::Task) -> AppResult<()> {
    let Some(cid) = t.countdown_id else { return Ok(()) };
    match t.due {
        None => {
            sqlx::query("UPDATE tasks SET countdown_id = NULL, days_before = NULL WHERE id = $1").bind(t.id).execute(db).await?;
        }
        Some(due) => {
            let c = countdowns::get(db, cid).await?;
            let n = (countdowns::local_date(&c, tz) - due).num_days().clamp(-365, 3650) as i32;
            sqlx::query("UPDATE tasks SET days_before = $2 WHERE id = $1").bind(t.id).bind(n).execute(db).await?;
        }
    }
    Ok(())
}

/// Milestones of the given countdowns, earliest first.
pub async fn for_countdowns(db: &PgPool, ids: &[Uuid]) -> AppResult<HashMap<Uuid, Vec<Milestone>>> {
    let rows: Vec<(Uuid, Uuid, String, i32, Option<NaiveDate>, bool)> = sqlx::query_as(
        "SELECT countdown_id, id, title, days_before, due, completed_at IS NOT NULL
         FROM tasks WHERE countdown_id = ANY($1) ORDER BY days_before DESC, created_at",
    )
    .bind(ids)
    .fetch_all(db)
    .await?;
    let mut out: HashMap<Uuid, Vec<Milestone>> = HashMap::new();
    for (cid, id, title, days_before, due, done) in rows {
        out.entry(cid).or_default().push(Milestone { id, title, days_before, due, done });
    }
    Ok(out)
}

/// Fill in `milestones` on a set of countdown views.
pub async fn attach(db: &PgPool, views: &mut [countdowns::CountdownView]) -> AppResult<()> {
    let ids: Vec<Uuid> = views.iter().map(|v| v.c.id).collect();
    let mut map = for_countdowns(db, &ids).await?;
    for v in views {
        v.milestones = map.remove(&v.c.id).unwrap_or_default();
    }
    Ok(())
}
