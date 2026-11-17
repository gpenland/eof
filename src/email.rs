//! Outbound email via Cloudflare Email Service's REST API. Used to deliver the daily news edition to the owner.

use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::json;

use crate::{config::EmailConfig, domain::news};

pub struct Mailer {
    http: reqwest::Client,
    cfg: EmailConfig,
}

pub struct Message {
    pub subject: String,
    pub html: String,
    pub text: String,
}

impl Mailer {
    pub fn new(cfg: EmailConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("EOF")
            .build()?;
        Ok(Self { http, cfg })
    }

    pub async fn send(&self, m: &Message) -> anyhow::Result<()> {
        let res = self
            .http
            .post(&self.cfg.api_url)
            .bearer_auth(&self.cfg.api_token)
            .json(&json!({
                "from": { "address": self.cfg.from, "name": self.cfg.from_name },
                "to": self.cfg.to,
                "subject": m.subject,
                "html": m.html,
                "text": m.text,
            }))
            .send()
            .await
            .context("reaching Cloudflare Email Service")?;
        let status = res.status();
        let body: Reply = match res.json().await {
            Ok(b) => b,
            Err(_) => bail!("Cloudflare returned {status} with an unreadable body"),
        };
        if !status.is_success() || !body.success {
            let why: Vec<String> = body.errors.iter().map(|e| format!("{} ({})", e.message, e.code)).collect();
            bail!("Cloudflare returned {status}: {}", if why.is_empty() { "no details".into() } else { why.join(", ") });
        }
        // Delivered or queued both count as sent; a permanent bounce means it never will be.
        if let Some(r) = body.result
            && !r.permanent_bounces.is_empty()
        {
            bail!("{} bounced permanently", r.permanent_bounces.join(", "));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
    result: Option<Outcome>,
}

#[derive(Deserialize)]
struct ApiError {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct Outcome {
    #[serde(default)]
    permanent_bounces: Vec<String>,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The edition as an email: a quiet masthead, the sanitized article, and a link back to the app.
pub fn issue_message(i: &news::Issue, origin: &str) -> Message {
    let date = i.edition.format("%A, %B %-d, %Y").to_string();
    let url = format!("{origin}/news/{}", i.edition);
    let body = news::render(&i.markdown);
    let title = escape(&i.title);
    let html = format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light"><title>{title}</title></head>
<body style="margin:0;padding:0;background:#f6f4ef;">
<div style="max-width:640px;margin:0 auto;padding:32px 20px 48px;font-family:Georgia,'Times New Roman',serif;font-size:18px;line-height:1.6;color:#1c1b19;">
  <div style="font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-size:12px;letter-spacing:.12em;text-transform:uppercase;color:#77736b;">{date}</div>
  <h1 style="font-size:34px;line-height:1.15;margin:8px 0 24px;font-weight:normal;">{title}</h1>
  <div style="border-top:1px solid #d9d5cc;padding-top:8px;">{body}</div>
  <p style="margin:40px 0 0;padding-top:16px;border-top:1px solid #d9d5cc;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-size:13px;color:#77736b;">
    EOF · <a href="{url}" style="color:#77736b;">Read in the app</a>
  </p>
</div>
</body></html>"#
    );
    let text = format!("{}\n{date}\n\n{}\n\n—\nRead in the app: {url}\n", i.title, i.markdown.trim());
    Message { subject: format!("{} — {}", i.title, i.edition.format("%b %-d")), html, text }
}
