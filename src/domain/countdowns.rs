use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    events::Actor,
};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Countdown {
    pub id: Uuid,
    pub title: String,
    pub at: DateTime<Utc>,
    pub all_day: bool,
    pub note: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

/// A countdown plus the human/AI-friendly derived fields.
#[derive(Debug, Serialize)]
pub struct CountdownView {
    #[serde(flatten)]
    pub c: Countdown,
    /// Local calendar date of the event (owner's timezone).
    pub date: NaiveDate,
    /// Local time, when the event isn't all-day.
    pub time: Option<String>,
    /// Whole calendar days from today; 0 = today, negative = in the past.
    pub days_until: i64,
    /// The countdown's T-minus plan (tasks scheduled relative to it). Filled by `milestones::attach`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<super::milestones::Milestone>,
}

#[derive(Debug, Deserialize)]
pub struct NewCountdown {
    pub title: String,
    pub date: NaiveDate,
    /// "HH:MM", local time. Omit for an all-day event.
    pub time: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Optional T-minus plan created along with the countdown.
    #[serde(default)]
    pub milestones: Vec<super::milestones::NewMilestone>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CountdownPatch {
    pub title: Option<String>,
    pub date: Option<NaiveDate>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub time: Option<Option<String>>,
    pub note: Option<String>,
}

fn parse_time(s: &str) -> AppResult<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s.trim(), "%H:%M:%S"))
        .map_err(|_| AppError::bad(format!("“{s}” isn't a time — use HH:MM (24h)")))
}

fn to_instant(tz: Tz, date: NaiveDate, time: Option<&str>) -> AppResult<(DateTime<Utc>, bool)> {
    let t = match time {
        Some(s) if !s.trim().is_empty() => parse_time(s)?,
        // All-day events are pure calendar dates: stored as midnight UTC and never shifted by
        // timezone changes. Clients count down to local midnight of that date.
        _ => return Ok((date.and_time(NaiveTime::MIN).and_utc(), true)),
    };
    let local = tz
        .from_local_datetime(&date.and_time(t))
        .earliest()
        .ok_or_else(|| AppError::bad("that local time doesn't exist (DST gap)"))?;
    Ok((local.with_timezone(&Utc), false))
}

/// Local calendar date of an event (all-day dates are timezone-free).
pub fn local_date(c: &Countdown, tz: Tz) -> NaiveDate {
    if c.all_day { c.at.date_naive() } else { c.at.with_timezone(&tz).date_naive() }
}

pub fn view(c: Countdown, tz: Tz, today: NaiveDate) -> CountdownView {
    let local = c.at.with_timezone(&tz);
    let date = local_date(&c, tz);
    let time = (!c.all_day).then(|| local.format("%H:%M").to_string());
    let days_until = (date - today).num_days();
    CountdownView { c, date, time, days_until, milestones: Vec::new() }
}

pub async fn list(db: &PgPool, include_past: bool) -> AppResult<Vec<Countdown>> {
    Ok(sqlx::query_as(
        "SELECT * FROM countdowns WHERE $1 OR at > now() - interval '2 days' ORDER BY at",
    )
    .bind(include_past)
    .fetch_all(db)
    .await?)
}

pub async fn get(db: &PgPool, id: Uuid) -> AppResult<Countdown> {
    sqlx::query_as("SELECT * FROM countdowns WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound("countdown"))
}

pub async fn create(db: &PgPool, tz: Tz, input: NewCountdown, actor: &Actor) -> AppResult<Countdown> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::bad("title can't be empty"));
    }
    let (at, all_day) = to_instant(tz, input.date, input.time.as_deref())?;
    Ok(sqlx::query_as(
        "INSERT INTO countdowns (id, title, at, all_day, note, created_by) VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(title)
    .bind(at)
    .bind(all_day)
    .bind(input.note.unwrap_or_default())
    .bind(actor.name())
    .fetch_one(db)
    .await?)
}

pub async fn update(db: &PgPool, tz: Tz, id: Uuid, p: CountdownPatch) -> AppResult<Countdown> {
    let c = get(db, id).await?;
    let local = c.at.with_timezone(&tz);
    let date = p.date.unwrap_or(local_date(&c, tz));
    let time = match p.time {
        Some(t) => t,
        None => (!c.all_day).then(|| local.format("%H:%M").to_string()),
    };
    let (at, all_day) = to_instant(tz, date, time.as_deref())?;
    let title = p.title.map(|t| t.trim().to_string()).unwrap_or(c.title);
    if title.is_empty() {
        return Err(AppError::bad("title can't be empty"));
    }
    Ok(sqlx::query_as("UPDATE countdowns SET title=$2, at=$3, all_day=$4, note=$5 WHERE id=$1 RETURNING *")
        .bind(id)
        .bind(title)
        .bind(at)
        .bind(all_day)
        .bind(p.note.unwrap_or(c.note))
        .fetch_one(db)
        .await?)
}

pub async fn delete(db: &PgPool, id: Uuid) -> AppResult<Countdown> {
    sqlx::query_as("DELETE FROM countdowns WHERE id = $1 RETURNING *")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound("countdown"))
}
