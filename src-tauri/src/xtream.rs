//! Minimal Xtream Codes API client: credential check + live channel fetch.
//!
//! Xtream panels expose a `player_api.php` endpoint. Authentication is
//! verified by checking `user_info.auth` in the JSON response; live channels
//! are fetched via the `get_live_categories` / `get_live_streams` actions
//! and their playable URLs are reconstructed as
//! `{base}/live/{user}/{pass}/{stream_id}.ts`.

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use url::Url;

/// Credentials for an Xtream Codes panel.
pub struct XtreamConfig {
    pub base: String,
    pub username: String,
    pub password: String,
}

/// A live stream as returned by the Xtream API, with a full playable URL.
pub struct XtreamChannel {
    pub name: String,
    pub url: String,
    pub logo_url: Option<String>,
    pub group: Option<String>,
}

/// Per-call timeout guard on top of the client's own timeouts.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Category {
    category_id: Option<String>,
    category_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Stream {
    stream_id: Option<i64>,
    name: Option<String>,
    stream_icon: Option<String>,
    category_id: Option<String>,
}

/// Build the `player_api.php` URL with the username/password query pair.
fn player_api_url(cfg: &XtreamConfig) -> Result<Url, String> {
    let base = cfg.base.trim_end_matches('/');
    let mut url = Url::parse(&format!("{base}/player_api.php"))
        .map_err(|e| format!("invalid Xtream base URL '{base}': {e}"))?;
    url.query_pairs_mut()
        .append_pair("username", cfg.username.as_str())
        .append_pair("password", cfg.password.as_str());
    Ok(url)
}

/// First 200 chars of a response body, for error messages.
fn excerpt(s: &str) -> String {
    s.trim().chars().take(200).collect()
}

/// GET a URL (with a 30s timeout) and decode the JSON body.
async fn get_json(client: &Client, url: Url) -> Result<serde_json::Value, String> {
    let resp = tokio::time::timeout(CALL_TIMEOUT, client.get(url).send())
        .await
        .map_err(|_| "request timed out".to_string())?
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("server returned HTTP {}", resp.status()));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("could not read response body: {e}"))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("unexpected response from server ({e}): {}", excerpt(&text)))
}

/// Validates credentials against the player_api; Err(String) with a friendly
/// message when auth fails.
pub async fn verify_auth(cfg: &XtreamConfig, client: &Client) -> Result<(), String> {
    let url = player_api_url(cfg)?;
    let json = get_json(client, url).await?;

    // Unexpected JSON shape (e.g. an HTML error page or a gateway error) is
    // treated as a failure, with a body excerpt in the message.
    let user_info = json
        .get("user_info")
        .ok_or_else(|| format!("unexpected response from server: {}", excerpt(&json.to_string())))?;
    let auth = user_info
        .get("auth")
        .ok_or_else(|| format!("unexpected response from server: {}", excerpt(&json.to_string())))?;

    // Panels report auth as `1`/`0`, `true`/`false`, or strings thereof.
    let ok = match auth {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_i64().map(|v| v != 0).unwrap_or(false),
        serde_json::Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    };
    if ok {
        return Ok(());
    }

    let status = user_info
        .get("status")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty());
    let hint = status.map(|s| format!(" (server says: {s})")).unwrap_or_default();
    Err(format!(
        "authentication failed for user '{}' - check username and password{hint}",
        cfg.username
    ))
}

/// Fetches live categories + live streams, returns channels with full stream
/// URLs.
pub async fn fetch_live(cfg: &XtreamConfig, client: &Client) -> Result<Vec<XtreamChannel>, String> {
    let base = cfg.base.trim_end_matches('/');

    // 1) category id -> name lookup
    let mut cat_url = player_api_url(cfg)?;
    cat_url
        .query_pairs_mut()
        .append_pair("action", "get_live_categories");
    let categories: Vec<Category> = serde_json::from_value(get_json(client, cat_url).await?)
        .map_err(|e| format!("unexpected categories response: {e}"))?;
    let mut cat_names: HashMap<String, String> = HashMap::new();
    for c in categories {
        if let (Some(id), Some(name)) = (c.category_id, c.category_name) {
            if !id.is_empty() && !name.is_empty() {
                cat_names.insert(id, name);
            }
        }
    }

    // 2) live streams
    let mut stream_url = player_api_url(cfg)?;
    stream_url
        .query_pairs_mut()
        .append_pair("action", "get_live_streams");
    let streams: Vec<Stream> = serde_json::from_value(get_json(client, stream_url).await?)
        .map_err(|e| format!("unexpected streams response: {e}"))?;

    let mut out = Vec::with_capacity(streams.len());
    for s in streams {
        // A stream without an id cannot be played.
        let Some(stream_id) = s.stream_id else {
            continue;
        };
        let name = s
            .name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Unknown".to_string());
        let url = live_url(base, &cfg.username, &cfg.password, stream_id);
        // Unknown or empty category ids map to no group.
        let group = s
            .category_id
            .as_ref()
            .and_then(|id| cat_names.get(id))
            .filter(|n| !n.is_empty())
            .cloned();
        out.push(XtreamChannel {
            name,
            url,
            logo_url: s.stream_icon.filter(|v| !v.is_empty()),
            group,
        });
    }
    Ok(out)
}

/// `{base}/live/{username}/{password}/{stream_id}.ts`
fn live_url(base: &str, username: &str, password: &str, stream_id: i64) -> String {
    let rel = format!("live/{username}/{password}/{stream_id}.ts");
    match Url::parse(base).and_then(|b| b.join(&rel)) {
        Ok(u) => u.to_string(),
        Err(_) => format!("{base}/live/{username}/{password}/{stream_id}.ts"),
    }
}
