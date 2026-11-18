//! Domain services. The web API and the MCP server are both thin adapters over these functions,
//! which is how "anything you can do in the UI, an AI can do too" stays true by construction.

pub mod countdowns;
pub mod milestones;
pub mod news;
pub mod quickadd;
pub mod tasks;

use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer};
use serde_json::json;
use sqlx::PgPool;

use crate::error::{AppError, AppResult};

/// The owner's timezone (set automatically from the browser). Defines what "today" means.
pub async fn timezone(db: &PgPool) -> Tz {
    let v: Option<serde_json::Value> = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'timezone'")
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    v.and_then(|v| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(Tz::UTC)
}

pub async fn set_timezone(db: &PgPool, tz: &str) -> AppResult<()> {
    let _: Tz = tz.parse().map_err(|_| AppError::bad(format!("unknown timezone {tz}")))?;
    sqlx::query("INSERT INTO settings (key, value) VALUES ('timezone', $1) ON CONFLICT (key) DO UPDATE SET value = $1")
        .bind(json!(tz))
        .execute(db)
        .await?;
    Ok(())
}

pub async fn today(db: &PgPool) -> NaiveDate {
    Utc::now().with_timezone(&timezone(db).await).date_naive()
}

/// Distinguishes "field absent" (None) from "field: null" (Some(None)) in PATCH bodies.
pub fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

/// Parse a date that may be written loosely by a person or a model: `2026-11-22`, `today`,
/// `tomorrow`, or a weekday name (the next one, counting today).
pub fn parse_date(s: &str, today: NaiveDate) -> AppResult<NaiveDate> {
    use chrono::{Datelike, Duration, Weekday};
    let t = s.trim().to_lowercase();
    match t.as_str() {
        "today" => return Ok(today),
        "tomorrow" => return Ok(today + Duration::days(1)),
        "yesterday" => return Ok(today - Duration::days(1)),
        _ => {}
    }
    if let Ok(wd) = t.parse::<Weekday>() {
        let mut d = today;
        while d.weekday() != wd {
            d += Duration::days(1);
        }
        return Ok(d);
    }
    NaiveDate::parse_from_str(&t, "%Y-%m-%d")
        .map_err(|_| AppError::bad(format!("“{s}” isn't a date — use YYYY-MM-DD")))
}
