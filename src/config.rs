use anyhow::{Context, bail};
use url::Url;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    /// Where people reach the app, e.g. `https://eof.example.com`. Passkeys are bound to its host.
    pub public_url: Url,
    pub port: u16,
    /// Optional fixed first-run setup code. When unset, one is generated and printed to the log.
    pub setup_code: Option<String>,
    /// Outbound email (Cloudflare Email Service). `None` when `CLOUDFLARE_EMAIL_TOKEN` is unset:
    /// email is simply off.
    pub email: Option<EmailConfig>,
}

#[derive(Clone, Debug)]
pub struct EmailConfig {
    /// Cloudflare API token with the "Email Sending: Edit" permission.
    pub api_token: String,
    /// `https://api.cloudflare.com/client/v4/accounts/{account}/email/sending/send`, or a local
    /// mock via `CLOUDFLARE_EMAIL_API_URL`.
    pub api_url: String,
    /// Sender address on a domain with Cloudflare Email Routing (or Email Sending) enabled.
    pub from: String,
    pub from_name: String,
    pub to: String,
}

impl EmailConfig {
    fn from_env() -> anyhow::Result<Option<Self>> {
        let get = |k: &str| std::env::var(k).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let Some(api_token) = get("CLOUDFLARE_EMAIL_TOKEN") else {
            return Ok(None);
        };
        let need = |k: &str| get(k).with_context(|| format!("{k} is required when CLOUDFLARE_EMAIL_TOKEN is set"));
        let api_url = match get("CLOUDFLARE_EMAIL_API_URL") {
            Some(u) => u,
            None => format!("https://api.cloudflare.com/client/v4/accounts/{}/email/sending/send", need("CLOUDFLARE_ACCOUNT_ID")?),
        };
        Ok(Some(Self {
            api_token,
            api_url,
            from: need("EMAIL_FROM")?,
            from_name: get("EMAIL_FROM_NAME").unwrap_or_else(|| "EOF".into()),
            to: need("EMAIL_TO")?,
        }))
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;
        let port = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
        let public_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| format!("http://localhost:{port}"));
        let mut public_url = Url::parse(&public_url).context("PUBLIC_URL must be a URL")?;
        public_url.set_path("");
        if public_url.host_str().is_none() {
            bail!("PUBLIC_URL needs a host");
        }
        if public_url.scheme() != "https" && public_url.host_str() != Some("localhost") {
            bail!("PUBLIC_URL must be https (only http://localhost is allowed for development)");
        }
        let setup_code = std::env::var("EOF_SETUP_CODE").ok().filter(|s| s.len() >= 8);
        let email = EmailConfig::from_env()?;
        Ok(Self { database_url, public_url, port, setup_code, email })
    }

    pub fn secure(&self) -> bool {
        self.public_url.scheme() == "https"
    }

    /// The public origin without a trailing slash.
    pub fn origin(&self) -> String {
        self.public_url.as_str().trim_end_matches('/').to_string()
    }

    pub fn cookie_name(&self) -> &'static str {
        // `__Host-` pins the cookie to this exact host over HTTPS with Path=/.
        if self.secure() { "__Host-eof" } else { "eof" }
    }
}
