//! The daily edition. Claude researches the news elsewhere and publishes plain Markdown here;
//! this module stores it (one issue per day) and renders it safely for reading.

use chrono::{DateTime, NaiveDate, Utc};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    events::Actor,
};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Issue {
    pub id: Uuid,
    pub edition: NaiveDate,
    pub title: String,
    pub markdown: String,
    pub read_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub published_at: DateTime<Utc>,
    pub emailed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct IssueSummary {
    pub id: Uuid,
    pub edition: NaiveDate,
    pub title: String,
    pub read: bool,
    pub minutes: u32,
    pub sections: Vec<String>,
    pub created_by: String,
}

#[derive(Debug, Serialize)]
pub struct Rendered {
    #[serde(flatten)]
    pub issue: Issue,
    pub html: String,
    pub sections: Vec<String>,
    pub minutes: u32,
    pub words: usize,
    pub prev: Option<NaiveDate>,
    pub next: Option<NaiveDate>,
}

fn opts() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_SMART_PUNCTUATION
}

/// Section names = the `##` headings.
pub fn sections(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for ev in Parser::new_ext(md, opts()) {
        match ev {
            Event::Start(Tag::Heading { level: HeadingLevel::H2, .. }) => cur = Some(String::new()),
            Event::End(TagEnd::Heading(HeadingLevel::H2)) => out.extend(cur.take()),
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = cur.as_mut() {
                    c.push_str(&t);
                }
            }
            _ => {}
        }
    }
    out
}

pub fn word_count(md: &str) -> usize {
    md.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

fn minutes(words: usize) -> u32 {
    ((words as f64 / 230.0).ceil() as u32).max(1)
}

pub fn render(md: &str) -> String {
    let mut raw = String::new();
    html::push_html(&mut raw, Parser::new_ext(md, opts()));
    // Markdown can carry raw HTML; sanitize everything before it reaches a browser.
    ammonia::Builder::default()
        .link_rel(Some("noopener noreferrer"))
        .url_schemes(["http", "https", "mailto"].into())
        .clean(&raw)
        .to_string()
}

/// If the Markdown opens with `# Title`, lift it out as the issue title.
fn split_title(md: &str) -> (Option<String>, String) {
    let trimmed = md.trim_start();
    if let Some(rest) = trimmed.strip_prefix("# ") {
        let (line, body) = rest.split_once('\n').unwrap_or((rest, ""));
        return (Some(line.trim().to_string()), body.trim_start().to_string());
    }
    (None, md.to_string())
}

pub async fn publish(
    db: &PgPool,
    edition: NaiveDate,
    title: Option<String>,
    markdown: &str,
    actor: &Actor,
) -> AppResult<Issue> {
    if markdown.trim().is_empty() {
        return Err(AppError::bad("markdown can't be empty"));
    }
    if markdown.len() > 400_000 {
        return Err(AppError::bad("issue is too long"));
    }
    let (md_title, body) = split_title(markdown);
    let title = title
        .filter(|t| !t.trim().is_empty())
        .or(md_title)
        .unwrap_or_else(|| "The Daily Brief".to_string());
    // Publishing the same day again replaces that day's issue (and marks it unread).
    Ok(sqlx::query_as(
        "INSERT INTO issues (id, edition, title, markdown, created_by) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (edition) DO UPDATE SET title = $3, markdown = $4, created_by = $5, read_at = NULL, published_at = now()
         RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(edition)
    .bind(title.trim())
    .bind(body)
    .bind(actor.name())
    .fetch_one(db)
    .await?)
}

pub async fn list(db: &PgPool, limit: i64) -> AppResult<Vec<IssueSummary>> {
    let rows: Vec<Issue> = sqlx::query_as("SELECT * FROM issues ORDER BY edition DESC LIMIT $1")
        .bind(limit.clamp(1, 1000))
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|i| IssueSummary {
            minutes: minutes(word_count(&i.markdown)),
            sections: sections(&i.markdown),
            id: i.id,
            edition: i.edition,
            title: i.title,
            read: i.read_at.is_some(),
            created_by: i.created_by,
        })
        .collect())
}

/// `which` is a YYYY-MM-DD edition date or "latest".
pub async fn get(db: &PgPool, which: &str) -> AppResult<Rendered> {
    let issue: Option<Issue> = if which == "latest" {
        sqlx::query_as("SELECT * FROM issues ORDER BY edition DESC LIMIT 1").fetch_optional(db).await?
    } else {
        let d = NaiveDate::parse_from_str(which, "%Y-%m-%d").map_err(|_| AppError::bad("edition must be YYYY-MM-DD or latest"))?;
        sqlx::query_as("SELECT * FROM issues WHERE edition = $1").bind(d).fetch_optional(db).await?
    };
    let issue = issue.ok_or(AppError::NotFound("issue"))?;
    let prev = sqlx::query_scalar("SELECT max(edition) FROM issues WHERE edition < $1").bind(issue.edition).fetch_one(db).await?;
    let next = sqlx::query_scalar("SELECT min(edition) FROM issues WHERE edition > $1").bind(issue.edition).fetch_one(db).await?;
    let words = word_count(&issue.markdown);
    Ok(Rendered {
        html: render(&issue.markdown),
        sections: sections(&issue.markdown),
        minutes: minutes(words),
        words,
        prev,
        next,
        issue,
    })
}

pub async fn set_read(db: &PgPool, edition: NaiveDate, read: bool) -> AppResult<()> {
    let r = sqlx::query("UPDATE issues SET read_at = CASE WHEN $2 THEN coalesce(read_at, now()) ELSE NULL END WHERE edition = $1")
        .bind(edition)
        .bind(read)
        .execute(db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("issue"));
    }
    Ok(())
}

/// Atomically mark an edition as emailed if it hasn't been yet; `None` means someone already did.
pub async fn claim_email(db: &PgPool, edition: NaiveDate) -> AppResult<Option<Issue>> {
    Ok(sqlx::query_as("UPDATE issues SET emailed_at = now() WHERE edition = $1 AND emailed_at IS NULL RETURNING *")
        .bind(edition)
        .fetch_optional(db)
        .await?)
}

/// Undo a claim after a failed send so a manual send (or retry) isn't blocked.
pub async fn release_email(db: &PgPool, edition: NaiveDate) -> AppResult<()> {
    sqlx::query("UPDATE issues SET emailed_at = NULL WHERE edition = $1").bind(edition).execute(db).await?;
    Ok(())
}

/// Like `claim_email`, for the push announcement: `Some(title)` only the first time.
pub async fn claim_push(db: &PgPool, edition: NaiveDate) -> AppResult<Option<String>> {
    Ok(sqlx::query_scalar("UPDATE issues SET pushed_at = now() WHERE edition = $1 AND pushed_at IS NULL RETURNING title")
        .bind(edition)
        .fetch_optional(db)
        .await?)
}

pub async fn mark_emailed(db: &PgPool, edition: NaiveDate) -> AppResult<Issue> {
    sqlx::query_as("UPDATE issues SET emailed_at = now() WHERE edition = $1 RETURNING *")
        .bind(edition)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound("issue"))
}

pub async fn delete(db: &PgPool, edition: NaiveDate) -> AppResult<Issue> {
    sqlx::query_as("DELETE FROM issues WHERE edition = $1 RETURNING *")
        .bind(edition)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound("issue"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifts_title_and_sections() {
        let (t, body) = split_title("# Morning Brief\n\n## World\nA.\n## Tech\nB.");
        assert_eq!(t.as_deref(), Some("Morning Brief"));
        assert_eq!(sections(&body), vec!["World", "Tech"]);
    }

    #[test]
    fn sanitizes_html() {
        let out = render("hi <script>alert(1)</script> [x](javascript:alert(1))");
        assert!(!out.contains("<script"));
        assert!(!out.contains("javascript:"));
    }
}
