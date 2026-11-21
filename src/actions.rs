//! Every state change goes through here: run the domain operation, then broadcast who did what.
//! The web API and the MCP tools both call these, so both surfaces behave identically.

use chrono::NaiveDate;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    domain::{self, countdowns, milestones, news, quickadd, tasks},
    email,
    error::{AppError, AppResult},
    events::{Actor, emit},
    push,
    state::AppState,
};

fn quote(s: &str) -> String {
    let s: String = s.chars().take(60).collect();
    format!("“{s}”")
}

pub async fn create_task(st: &AppState, a: &Actor, input: tasks::NewTask) -> AppResult<tasks::Task> {
    let today = domain::today(&st.db).await;
    let t = tasks::create(&st.db, input, a, today).await?;
    emit(st, "tasks", a, format!("added {}", quote(&t.title)));
    Ok(t)
}

pub async fn update_task(st: &AppState, a: &Actor, id: Uuid, p: tasks::TaskPatch) -> AppResult<tasks::Task> {
    let today = domain::today(&st.db).await;
    let due_changed = p.due.is_some();
    let mut t = tasks::update(&st.db, id, p, today).await?;
    if due_changed && t.countdown_id.is_some() {
        milestones::rebase(&st.db, domain::timezone(&st.db).await, &t).await?;
        t = tasks::get(&st.db, id).await?;
    }
    emit(st, "tasks", a, format!("edited {}", quote(&t.title)));
    Ok(t)
}

pub async fn complete_task(st: &AppState, a: &Actor, id: Uuid) -> AppResult<Value> {
    let today = domain::today(&st.db).await;
    Ok(match tasks::complete(&st.db, id, today).await? {
        tasks::Completion::Done(t) => {
            emit(st, "tasks", a, format!("completed {}", quote(&t.title)));
            json!({ "task": t, "rolled_to": null })
        }
        tasks::Completion::Rolled { task, next } => {
            emit(st, "tasks", a, format!("completed {} — next {}", quote(&task.title), next.format("%b %-d")));
            json!({ "task": task, "rolled_to": next })
        }
    })
}

/// Add a task from one sentence ("call dentist fri #health !"), parsed like the web composer.
pub async fn quick_add(st: &AppState, a: &Actor, text: &str, list_id: Option<Uuid>) -> AppResult<tasks::Task> {
    let today = domain::today(&st.db).await;
    let p = quickadd::parse(text, today);
    if p.title.is_empty() {
        return Err(AppError::bad("that's all date and tags — what's the task?"));
    }
    let input = tasks::NewTask {
        list_id,
        title: p.title,
        tags: Some(p.tags),
        due: p.due,
        repeat: p.repeat.map(String::from),
        starred: Some(p.starred),
        notes: None,
    };
    create_task(st, a, input).await
}

pub async fn uncomplete_task(st: &AppState, a: &Actor, id: Uuid) -> AppResult<tasks::Task> {
    let t = tasks::uncomplete(&st.db, id).await?;
    emit(st, "tasks", a, format!("reopened {}", quote(&t.title)));
    Ok(t)
}

pub async fn delete_task(st: &AppState, a: &Actor, id: Uuid) -> AppResult<tasks::Task> {
    let t = tasks::delete(&st.db, id).await?;
    emit(st, "tasks", a, format!("deleted {}", quote(&t.title)));
    Ok(t)
}

pub async fn move_task(st: &AppState, a: &Actor, id: Uuid, after: Option<Uuid>) -> AppResult<tasks::Task> {
    let t = tasks::move_after(&st.db, id, after).await?;
    emit(st, "tasks", a, format!("reordered {}", quote(&t.title)));
    Ok(t)
}

pub async fn clear_completed(st: &AppState, a: &Actor, list_id: Uuid) -> AppResult<u64> {
    let n = tasks::clear_completed(&st.db, list_id).await?;
    emit(st, "tasks", a, format!("cleared {n} completed"));
    Ok(n)
}

pub async fn create_list(st: &AppState, a: &Actor, name: &str) -> AppResult<tasks::TaskList> {
    let l = tasks::create_list(&st.db, name).await?;
    emit(st, "tasks", a, format!("created list {}", quote(&l.name)));
    Ok(l)
}

pub async fn rename_list(st: &AppState, a: &Actor, id: Uuid, name: &str) -> AppResult<()> {
    tasks::rename_list(&st.db, id, name).await?;
    emit(st, "tasks", a, format!("renamed a list to {}", quote(name)));
    Ok(())
}

pub async fn delete_list(st: &AppState, a: &Actor, id: Uuid) -> AppResult<()> {
    let name = tasks::delete_list(&st.db, id).await?;
    emit(st, "tasks", a, format!("deleted list {}", quote(&name)));
    Ok(())
}

pub async fn create_countdown(st: &AppState, a: &Actor, mut input: countdowns::NewCountdown) -> AppResult<countdowns::CountdownView> {
    let tz = domain::timezone(&st.db).await;
    let plan = std::mem::take(&mut input.milestones);
    for m in &plan {
        tasks::check_title(&m.title)?;
    }
    let c = countdowns::create(&st.db, tz, input, a).await?;
    for m in plan {
        milestones::add(&st.db, tz, c.id, m, a).await?;
    }
    emit(st, "countdowns", a, format!("added countdown {}", quote(&c.title)));
    let mut v = [countdowns::view(c, tz, domain::today(&st.db).await)];
    milestones::attach(&st.db, &mut v).await?;
    let [v] = v;
    Ok(v)
}

/// Add a step to a countdown's T-minus plan: a task due `days_before` days ahead of it.
pub async fn add_milestone(st: &AppState, a: &Actor, countdown_id: Uuid, m: milestones::NewMilestone) -> AppResult<tasks::Task> {
    let tz = domain::timezone(&st.db).await;
    let t = milestones::add(&st.db, tz, countdown_id, m, a).await?;
    let c = countdowns::get(&st.db, countdown_id).await?;
    let when = match t.days_before.unwrap_or(0) {
        0 => "on the day".to_string(),
        n if n > 0 => format!("T-{n}"),
        n => format!("{} after", -n),
    };
    emit(st, "tasks", a, format!("planned {} for {} ({when})", quote(&t.title), quote(&c.title)));
    Ok(t)
}

pub async fn update_countdown(
    st: &AppState,
    a: &Actor,
    id: Uuid,
    p: countdowns::CountdownPatch,
) -> AppResult<countdowns::CountdownView> {
    let tz = domain::timezone(&st.db).await;
    let c = countdowns::update(&st.db, tz, id, p).await?;
    let moved = milestones::follow(&st.db, tz, &c).await?;
    let also = if moved > 0 { format!(" — moved {moved} planned task{}", if moved == 1 { "" } else { "s" }) } else { String::new() };
    emit(st, "countdowns", a, format!("edited countdown {}{also}", quote(&c.title)));
    if moved > 0 {
        emit(st, "tasks", a, format!("rescheduled the plan for {}", quote(&c.title)));
    }
    let mut v = [countdowns::view(c, tz, domain::today(&st.db).await)];
    milestones::attach(&st.db, &mut v).await?;
    let [v] = v;
    Ok(v)
}

pub async fn delete_countdown(st: &AppState, a: &Actor, id: Uuid) -> AppResult<()> {
    let c = countdowns::delete(&st.db, id).await?;
    emit(st, "countdowns", a, format!("removed countdown {}", quote(&c.title)));
    Ok(())
}

pub async fn publish_issue(
    st: &AppState,
    a: &Actor,
    edition: Option<NaiveDate>,
    title: Option<String>,
    markdown: &str,
) -> AppResult<news::Issue> {
    let edition = match edition {
        Some(d) => d,
        None => domain::today(&st.db).await,
    };
    let i = news::publish(&st.db, edition, title, markdown, a).await?;
    emit(st, "news", a, format!("published {} ({})", quote(&i.title), i.edition.format("%b %-d")));
    // Today's edition is emailed once, on first publish. It runs in the background so a slow or
    // failing mail provider never holds up (or fails) the publish; the outcome arrives as an event.
    let is_today = i.edition == domain::today(&st.db).await;
    if st.mailer.is_some() && is_today {
        tokio::spawn(auto_email(st.clone(), a.clone(), i.edition));
    }
    // Announce today's edition on devices with notifications on — once, like the email.
    if is_today && let Some(title) = news::claim_push(&st.db, i.edition).await? {
        push::spawn_send(st, push::Note { title: "Today’s edition is in".into(), body: title, url: "/news".into(), tag: "edition".into() });
    }
    Ok(i)
}

async fn auto_email(st: AppState, a: Actor, edition: NaiveDate) {
    let i = match news::claim_email(&st.db, edition).await {
        Ok(Some(i)) => i,
        Ok(None) => return, // already emailed (this is a republish)
        Err(e) => return tracing::error!("claiming {edition} for email: {e}"),
    };
    let Some(mailer) = &st.mailer else { return };
    let token = match news::read_token(&st.db, edition).await {
        Ok(t) => t,
        Err(e) => {
            let _ = news::release_email(&st.db, edition).await;
            return tracing::error!("minting the {edition} read token: {e}");
        }
    };
    match mailer.send(&email::issue_message(&i, &st.cfg.origin(), &token)).await {
        Ok(()) => emit(&st, "news", &a, format!("emailed {} to you", quote(&i.title))),
        Err(e) => {
            tracing::warn!("emailing the {edition} edition failed: {e:#}");
            let _ = news::release_email(&st.db, edition).await;
            emit(&st, "news", &a, format!("couldn’t email {}: {e}", quote(&i.title)));
        }
    }
}

/// Send an edition by email now, whether or not it was emailed before.
pub async fn email_issue(st: &AppState, a: &Actor, edition: NaiveDate) -> AppResult<news::Issue> {
    let mailer = st.mailer.as_ref().ok_or_else(|| AppError::bad("email isn’t set up (CLOUDFLARE_EMAIL_TOKEN, CLOUDFLARE_ACCOUNT_ID, EMAIL_FROM, EMAIL_TO)"))?;
    let i = news::get(&st.db, &edition.to_string()).await?.issue;
    let token = news::read_token(&st.db, edition).await?;
    mailer
        .send(&email::issue_message(&i, &st.cfg.origin(), &token))
        .await
        .map_err(|e| AppError::bad(format!("couldn’t send: {e}")))?;
    let i = news::mark_emailed(&st.db, edition).await?;
    emit(st, "news", a, format!("emailed {} to you", quote(&i.title)));
    Ok(i)
}

pub async fn set_issue_read(st: &AppState, a: &Actor, edition: NaiveDate, read: bool) -> AppResult<()> {
    news::set_read(&st.db, edition, read).await?;
    emit(st, "news", a, if read { "marked an issue read" } else { "marked an issue unread" });
    Ok(())
}

/// The emailed edition's image was loaded: the owner opened it in their inbox.
pub async fn read_issue_by_email(st: &AppState, token: &str) -> AppResult<()> {
    if let Some(i) = news::read_by_token(&st.db, token).await? {
        emit(st, "news", &Actor::you(), format!("read {} in email", quote(&i.title)));
    }
    Ok(())
}

pub async fn delete_issue(st: &AppState, a: &Actor, edition: NaiveDate) -> AppResult<()> {
    let i = news::delete(&st.db, edition).await?;
    emit(st, "news", a, format!("deleted the {} issue", i.edition.format("%b %-d")));
    Ok(())
}

/// A compact picture of "now": what's overdue, what's today, what's coming, what to read.
pub async fn overview(st: &AppState) -> AppResult<Value> {
    let db = &st.db;
    let tz = domain::timezone(db).await;
    let today = domain::today(db).await;
    let open = tasks::query(db, &tasks::TaskQuery { due_before: Some(today + chrono::Duration::days(7)), ..Default::default() }).await?;
    let pick = |f: &dyn Fn(NaiveDate) -> bool| -> Vec<Value> {
        open.iter()
            .filter(|t| t.due.map(f).unwrap_or(false))
            .map(|t| json!({ "id": t.id, "title": t.title, "due": t.due, "tags": t.tags, "starred": t.starred, "countdown": t.countdown_title }))
            .collect()
    };
    let overdue = pick(&|d| d < today);
    let due_today = pick(&|d| d == today);
    let this_week = pick(&|d| d > today);
    let cds: Vec<_> = countdowns::list(db, false)
        .await?
        .into_iter()
        .take(5)
        .map(|c| countdowns::view(c, tz, today))
        .collect();
    let latest = news::list(db, 1).await?.into_iter().next();
    // What got finished yesterday (owner's calendar day), for the morning recap.
    let finished: Vec<String> = sqlx::query_scalar(
        "SELECT title FROM tasks WHERE completed_at IS NOT NULL AND (completed_at AT TIME ZONE $1)::date = $2 ORDER BY completed_at",
    )
    .bind(tz.name())
    .bind(today - chrono::Duration::days(1))
    .fetch_all(db)
    .await?;
    Ok(json!({
        "today": today,
        "timezone": tz.name(),
        "tasks": { "overdue": overdue, "today": due_today, "next_7_days": this_week },
        "done_yesterday": { "count": finished.len(), "titles": finished.iter().take(6).collect::<Vec<_>>() },
        "countdowns": cds,
        "latest_issue": latest.map(|i| json!({ "edition": i.edition, "title": i.title, "read": i.read, "is_today": i.edition == today })),
    }))
}
