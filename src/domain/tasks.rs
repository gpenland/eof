use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc, Weekday};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::double_option;
use crate::{
    error::{AppError, AppResult},
    events::Actor,
};

pub const REPEATS: [&str; 5] = ["daily", "weekdays", "weekly", "monthly", "yearly"];

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Task {
    pub id: Uuid,
    pub list_id: Uuid,
    pub title: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub due: Option<NaiveDate>,
    pub repeat: Option<String>,
    pub starred: bool,
    pub position: f64,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TaskList {
    pub id: Uuid,
    pub name: String,
    pub position: f64,
    pub open_count: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct NewTask {
    pub list_id: Option<Uuid>,
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub due: Option<NaiveDate>,
    #[serde(default)]
    pub repeat: Option<String>,
    #[serde(default)]
    pub starred: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub due: Option<Option<NaiveDate>>,
    #[serde(default, deserialize_with = "double_option")]
    pub repeat: Option<Option<String>>,
    pub starred: Option<bool>,
    pub list_id: Option<Uuid>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TaskQuery {
    pub list_id: Option<Uuid>,
    /// open (default) | completed | all
    pub status: Option<String>,
    pub tag: Option<String>,
    pub search: Option<String>,
    pub due_before: Option<NaiveDate>,
    pub due_after: Option<NaiveDate>,
    pub limit: Option<i64>,
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t: String = t
            .trim()
            .trim_start_matches('#')
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        if !t.is_empty() && t.len() <= 40 && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn check_repeat(r: &Option<String>) -> AppResult<()> {
    match r {
        Some(r) if !REPEATS.contains(&r.as_str()) => {
            Err(AppError::bad(format!("repeat must be one of {}", REPEATS.join(", "))))
        }
        _ => Ok(()),
    }
}

fn check_title(t: &str) -> AppResult<String> {
    let t = t.trim();
    if t.is_empty() {
        return Err(AppError::bad("title can't be empty"));
    }
    if t.len() > 500 {
        return Err(AppError::bad("title is too long"));
    }
    Ok(t.to_string())
}

// ── Lists ──────────────────────────────────────────────────────────────────

pub async fn ensure_default_list(db: &PgPool) -> anyhow::Result<()> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM task_lists").fetch_one(db).await?;
    if n == 0 {
        sqlx::query("INSERT INTO task_lists (id, name, position) VALUES ($1, 'To-Do', 0)")
            .bind(Uuid::new_v4())
            .execute(db)
            .await?;
    }
    Ok(())
}

pub async fn lists(db: &PgPool) -> AppResult<Vec<TaskList>> {
    Ok(sqlx::query_as(
        "SELECT l.id, l.name, l.position,
                (SELECT count(*) FROM tasks t WHERE t.list_id = l.id AND t.completed_at IS NULL) AS open_count
         FROM task_lists l ORDER BY l.position, l.created_at",
    )
    .fetch_all(db)
    .await?)
}

pub async fn default_list(db: &PgPool) -> AppResult<Uuid> {
    Ok(sqlx::query_scalar("SELECT id FROM task_lists ORDER BY position, created_at LIMIT 1").fetch_one(db).await?)
}

pub async fn create_list(db: &PgPool, name: &str) -> AppResult<TaskList> {
    let name = check_title(name)?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO task_lists (id, name, position) VALUES ($1, $2, (SELECT coalesce(max(position), 0) + 1 FROM task_lists))")
        .bind(id)
        .bind(&name)
        .execute(db)
        .await?;
    Ok(TaskList { id, name, position: 0.0, open_count: 0 })
}

pub async fn rename_list(db: &PgPool, id: Uuid, name: &str) -> AppResult<()> {
    let name = check_title(name)?;
    let r = sqlx::query("UPDATE task_lists SET name = $2 WHERE id = $1").bind(id).bind(name).execute(db).await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("list"));
    }
    Ok(())
}

pub async fn delete_list(db: &PgPool, id: Uuid) -> AppResult<String> {
    if lists(db).await?.len() <= 1 {
        return Err(AppError::bad("You need at least one list."));
    }
    let name: String = sqlx::query_scalar("DELETE FROM task_lists WHERE id = $1 RETURNING name")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound("list"))?;
    Ok(name)
}

// ── Tasks ──────────────────────────────────────────────────────────────────

pub async fn get(db: &PgPool, id: Uuid) -> AppResult<Task> {
    sqlx::query_as("SELECT * FROM tasks WHERE id = $1").bind(id).fetch_optional(db).await?.ok_or(AppError::NotFound("task"))
}

pub async fn query(db: &PgPool, q: &TaskQuery) -> AppResult<Vec<Task>> {
    let status = q.status.as_deref().unwrap_or("open");
    if !["open", "completed", "all"].contains(&status) {
        return Err(AppError::bad("status must be open, completed or all"));
    }
    let tag = q.tag.as_ref().map(|t| normalize_tags(std::slice::from_ref(t))).and_then(|v| v.into_iter().next());
    Ok(sqlx::query_as(
        "SELECT * FROM tasks
         WHERE ($1::uuid IS NULL OR list_id = $1)
           AND ($2 = 'all' OR ($2 = 'open' AND completed_at IS NULL) OR ($2 = 'completed' AND completed_at IS NOT NULL))
           AND ($3::text IS NULL OR $3 = ANY(tags))
           AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR notes ILIKE '%' || $4 || '%')
           AND ($5::date IS NULL OR due <= $5)
           AND ($6::date IS NULL OR due >= $6)
         ORDER BY completed_at DESC NULLS FIRST, due ASC NULLS LAST, position
         LIMIT $7",
    )
    .bind(q.list_id)
    .bind(status)
    .bind(tag)
    .bind(q.search.as_deref().filter(|s| !s.trim().is_empty()))
    .bind(q.due_before)
    .bind(q.due_after)
    .bind(q.limit.unwrap_or(500).clamp(1, 2000))
    .fetch_all(db)
    .await?)
}

pub async fn all_tags(db: &PgPool) -> AppResult<Vec<(String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT tag, count(*) FROM tasks, unnest(tags) AS tag WHERE completed_at IS NULL GROUP BY tag ORDER BY count(*) DESC, tag",
    )
    .fetch_all(db)
    .await?)
}

pub async fn create(db: &PgPool, input: NewTask, actor: &Actor, today: NaiveDate) -> AppResult<Task> {
    let title = check_title(&input.title)?;
    check_repeat(&input.repeat)?;
    let list_id = match input.list_id {
        Some(id) => id,
        None => default_list(db).await?,
    };
    let tags = normalize_tags(&input.tags.unwrap_or_default());
    // New tasks go to the top of "My order", like Google Tasks.
    let task: Task = sqlx::query_as(
        "INSERT INTO tasks (id, list_id, title, notes, tags, due, repeat, starred, position, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                 (SELECT coalesce(min(position), 1) - 1 FROM tasks WHERE list_id = $2 AND completed_at IS NULL), $9)
         RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(list_id)
    .bind(title)
    .bind(input.notes.unwrap_or_default())
    .bind(tags)
    // A repeating task needs an anchor date; default it to today.
    .bind(input.due.or(input.repeat.as_ref().map(|_| today)))
    .bind(&input.repeat)
    .bind(input.starred.unwrap_or(false))
    .bind(actor.name())
    .fetch_one(db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref d) if d.is_foreign_key_violation() => AppError::NotFound("list"),
        e => e.into(),
    })?;
    Ok(task)
}

pub async fn update(db: &PgPool, id: Uuid, p: TaskPatch, today: NaiveDate) -> AppResult<Task> {
    let mut t = get(db, id).await?;
    if let Some(title) = p.title {
        t.title = check_title(&title)?;
    }
    if let Some(notes) = p.notes {
        t.notes = notes;
    }
    if let Some(tags) = p.tags {
        t.tags = normalize_tags(&tags);
    }
    if let Some(due) = p.due {
        t.due = due;
    }
    if let Some(repeat) = p.repeat {
        check_repeat(&repeat)?;
        t.repeat = repeat;
    }
    // A repeating task needs an anchor date.
    if t.repeat.is_some() && t.due.is_none() {
        t.due = Some(today);
    }
    if let Some(s) = p.starred {
        t.starred = s;
    }
    let moved_list = p.list_id.filter(|l| *l != t.list_id);
    if let Some(l) = moved_list {
        t.list_id = l;
    }
    let t: Task = sqlx::query_as(
        "UPDATE tasks SET title=$2, notes=$3, tags=$4, due=$5, repeat=$6, starred=$7, list_id=$8, updated_at=now(),
            position = CASE WHEN $9 THEN (SELECT coalesce(min(position), 1) - 1 FROM tasks WHERE list_id = $8) ELSE position END
         WHERE id=$1 RETURNING *",
    )
    .bind(id)
    .bind(&t.title)
    .bind(&t.notes)
    .bind(&t.tags)
    .bind(t.due)
    .bind(&t.repeat)
    .bind(t.starred)
    .bind(t.list_id)
    .bind(moved_list.is_some())
    .fetch_one(db)
    .await?;
    Ok(t)
}

pub fn next_occurrence(from: NaiveDate, rule: &str) -> NaiveDate {
    match rule {
        "daily" => from + Duration::days(1),
        "weekdays" => {
            let mut d = from + Duration::days(1);
            while matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                d += Duration::days(1);
            }
            d
        }
        "weekly" => from + Duration::days(7),
        "monthly" => from.checked_add_months(Months::new(1)).unwrap_or(from + Duration::days(30)),
        "yearly" => from.checked_add_months(Months::new(12)).unwrap_or(from + Duration::days(365)),
        _ => from,
    }
}

pub enum Completion {
    Done(Task),
    /// A repeating task: this occurrence was logged and the task rolled forward.
    Rolled { task: Task, next: NaiveDate },
}

pub async fn complete(db: &PgPool, id: Uuid, today: NaiveDate) -> AppResult<Completion> {
    let t = get(db, id).await?;
    if t.completed_at.is_some() {
        return Ok(Completion::Done(t));
    }
    if let (Some(rule), Some(due)) = (t.repeat.as_deref(), t.due) {
        let mut next = next_occurrence(due, rule);
        while next < today {
            next = next_occurrence(next, rule);
        }
        let mut tx = db.begin().await?;
        // Log the finished occurrence as its own completed task, so history stays honest.
        sqlx::query(
            "INSERT INTO tasks (id, list_id, title, notes, tags, due, repeat, starred, position, completed_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, $8, now(), $9)",
        )
        .bind(Uuid::new_v4())
        .bind(t.list_id)
        .bind(&t.title)
        .bind(&t.notes)
        .bind(&t.tags)
        .bind(t.due)
        .bind(t.starred)
        .bind(t.position)
        .bind(&t.created_by)
        .execute(&mut *tx)
        .await?;
        let task: Task = sqlx::query_as("UPDATE tasks SET due = $2, updated_at = now() WHERE id = $1 RETURNING *")
            .bind(id)
            .bind(next)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Completion::Rolled { task, next });
    }
    let task = sqlx::query_as("UPDATE tasks SET completed_at = now(), updated_at = now() WHERE id = $1 RETURNING *")
        .bind(id)
        .fetch_one(db)
        .await?;
    Ok(Completion::Done(task))
}

pub async fn uncomplete(db: &PgPool, id: Uuid) -> AppResult<Task> {
    sqlx::query_as("UPDATE tasks SET completed_at = NULL, updated_at = now() WHERE id = $1 RETURNING *")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound("task"))
}

pub async fn delete(db: &PgPool, id: Uuid) -> AppResult<Task> {
    sqlx::query_as("DELETE FROM tasks WHERE id = $1 RETURNING *")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound("task"))
}

pub async fn clear_completed(db: &PgPool, list_id: Uuid) -> AppResult<u64> {
    Ok(sqlx::query("DELETE FROM tasks WHERE list_id = $1 AND completed_at IS NOT NULL")
        .bind(list_id)
        .execute(db)
        .await?
        .rows_affected())
}

/// Custom ordering: place `id` directly after `after` (or at the top when `after` is None).
/// Uses fractional positions; renumbers the list if the gap gets too small.
pub async fn move_after(db: &PgPool, id: Uuid, after: Option<Uuid>) -> AppResult<Task> {
    let t = get(db, id).await?;
    let positions: Vec<(Uuid, f64)> = sqlx::query_as(
        "SELECT id, position FROM tasks WHERE list_id = $1 AND completed_at IS NULL AND id <> $2 ORDER BY position",
    )
    .bind(t.list_id)
    .bind(id)
    .fetch_all(db)
    .await?;
    let idx = match after {
        None => 0,
        Some(a) => positions.iter().position(|(pid, _)| *pid == a).ok_or(AppError::bad("`after` task isn't open in the same list"))? + 1,
    };
    let prev = idx.checked_sub(1).map(|i| positions[i].1);
    let next = positions.get(idx).map(|p| p.1);
    let pos = match (prev, next) {
        (None, None) => 0.0,
        (None, Some(n)) => n - 1.0,
        (Some(p), None) => p + 1.0,
        (Some(p), Some(n)) => (p + n) / 2.0,
    };
    let tight = matches!((prev, next), (Some(p), Some(n)) if (n - p).abs() < 1e-6);
    if tight {
        let mut tx = db.begin().await?;
        let mut order: Vec<Uuid> = positions.iter().map(|p| p.0).collect();
        order.insert(idx, id);
        for (i, tid) in order.iter().enumerate() {
            sqlx::query("UPDATE tasks SET position = $2 WHERE id = $1").bind(tid).bind(i as f64).execute(&mut *tx).await?;
        }
        tx.commit().await?;
    } else {
        sqlx::query("UPDATE tasks SET position = $2 WHERE id = $1").bind(id).bind(pos).execute(db).await?;
    }
    get(db, id).await
}
