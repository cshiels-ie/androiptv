//! Minimal Xtream Codes API client: credential check + live / VOD / series
//! fetch.
//!
//! Xtream panels expose a `player_api.php` endpoint. Authentication is
//! verified by checking `user_info.auth` in the JSON response; live channels
//! are fetched via the `get_live_categories` / `get_live_streams` actions
//! and their playable URLs are reconstructed as
//! `{base}/live/{user}/{pass}/{stream_id}.ts`. VOD movies use
//! `{base}/movie/{user}/{pass}/{stream_id}.{ext}`; series episodes are
//! fetched lazily via `get_series_info` as
//! `{base}/series/{user}/{pass}/{series_id}/{season}/{ep}.{ext}`.

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

/// A VOD movie or series row, with a full playable URL (movies) or an empty
/// URL (series — episodes are fetched on demand) plus the remote id needed
/// to build episode URLs later.
pub struct XtreamItem {
    pub name: String,
    pub url: String,
    pub logo_url: Option<String>,
    pub group: Option<String>,
    pub remote_id: Option<String>,
}

/// A single series episode with a full playable URL.
pub struct RawEpisode {
    pub season: i64,
    pub episode_num: i64,
    pub title: String,
    pub url: String,
    pub logo_url: Option<String>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct VodStream {
    stream_id: Option<i64>,
    name: Option<String>,
    stream_icon: Option<String>,
    category_id: Option<String>,
    container_extension: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SeriesItem {
    series_id: Option<i64>,
    name: Option<String>,
    cover: Option<String>,
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

/// `{base}/{kind}/{username}/{password}/{parts}.{ext}` — kind is one of
/// `live`, `movie`, `series`.
fn media_url(base: &str, kind: &str, username: &str, password: &str, parts: &str, ext: &str) -> String {
    let rel = format!("{kind}/{username}/{password}/{parts}.{ext}");
    match Url::parse(base).and_then(|b| b.join(&rel)) {
        Ok(u) => u.to_string(),
        Err(_) => format!("{base}/{kind}/{username}/{password}/{parts}.{ext}"),
    }
}

/// `{base}/live/{username}/{password}/{stream_id}.ts`
fn live_url(base: &str, username: &str, password: &str, stream_id: i64) -> String {
    media_url(base, "live", username, password, &stream_id.to_string(), "ts")
}

/// Fetches the category id → name map for an action, tolerating panels
/// that answer with an error object instead of a list (treated as empty —
/// a missing VOD/series section must not fail the whole import).
async fn fetch_category_names(
    cfg: &XtreamConfig,
    client: &Client,
    action: &str,
) -> Result<HashMap<String, String>, String> {
    let mut url = player_api_url(cfg)?;
    url.query_pairs_mut().append_pair("action", action);
    let json = get_json(client, url).await?;
    if !json.is_array() {
        return Ok(HashMap::new());
    }
    let categories: Vec<Category> = serde_json::from_value(json)
        .map_err(|e| format!("unexpected categories response: {e}"))?;
    let mut cat_names = HashMap::new();
    for c in categories {
        if let (Some(id), Some(name)) = (c.category_id, c.category_name) {
            if !id.is_empty() && !name.is_empty() {
                cat_names.insert(id, name);
            }
        }
    }
    Ok(cat_names)
}

/// Fetches VOD movies: `{base}/movie/{user}/{pass}/{stream_id}.{ext}`.
/// Panels without VOD support return an error object — treated as empty.
pub async fn fetch_vod(cfg: &XtreamConfig, client: &Client) -> Result<Vec<XtreamItem>, String> {
    let base = cfg.base.trim_end_matches('/');
    let cat_names = fetch_category_names(cfg, client, "get_vod_categories").await?;

    let mut url = player_api_url(cfg)?;
    url.query_pairs_mut().append_pair("action", "get_vod_streams");
    let json = get_json(client, url).await?;
    let streams: Vec<VodStream> = if json.is_array() {
        serde_json::from_value(json).map_err(|e| format!("unexpected vod streams response: {e}"))?
    } else {
        vec![]
    };

    let mut out = Vec::with_capacity(streams.len());
    for s in streams {
        // A stream without an id cannot be played.
        let Some(stream_id) = s.stream_id else {
            continue;
        };
        let name = s.name.filter(|n| !n.is_empty()).unwrap_or_else(|| "Unknown".to_string());
        let ext = s
            .container_extension
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "mp4".to_string());
        let url = media_url(base, "movie", &cfg.username, &cfg.password, &stream_id.to_string(), &ext);
        let group = s
            .category_id
            .as_ref()
            .and_then(|id| cat_names.get(id))
            .filter(|n| !n.is_empty())
            .cloned();
        out.push(XtreamItem {
            name,
            url,
            logo_url: s.stream_icon.filter(|v| !v.is_empty()),
            group,
            remote_id: Some(stream_id.to_string()),
        });
    }
    Ok(out)
}

/// Fetches series rows. Series have no single stream URL (episodes are
/// fetched on demand), so `url` is empty; `remote_id` carries the panel's
/// series_id needed for `get_series_info`.
pub async fn fetch_series(cfg: &XtreamConfig, client: &Client) -> Result<Vec<XtreamItem>, String> {
    let cat_names = fetch_category_names(cfg, client, "get_series_categories").await?;

    let mut url = player_api_url(cfg)?;
    url.query_pairs_mut().append_pair("action", "get_series");
    let json = get_json(client, url).await?;
    let series: Vec<SeriesItem> = if json.is_array() {
        serde_json::from_value(json).map_err(|e| format!("unexpected series response: {e}"))?
    } else {
        vec![]
    };

    let mut out = Vec::with_capacity(series.len());
    for s in series {
        let Some(series_id) = s.series_id else {
            continue;
        };
        let name = s.name.filter(|n| !n.is_empty()).unwrap_or_else(|| "Unknown".to_string());
        let group = s
            .category_id
            .as_ref()
            .and_then(|id| cat_names.get(id))
            .filter(|n| !n.is_empty())
            .cloned();
        out.push(XtreamItem {
            name,
            url: String::new(),
            logo_url: s.cover.filter(|v| !v.is_empty()),
            group,
            remote_id: Some(series_id.to_string()),
        });
    }
    Ok(out)
}

/// Fetches the season/episode list for a series via `get_series_info`,
/// materializing each episode URL.
pub async fn fetch_series_episodes(
    cfg: &XtreamConfig,
    client: &Client,
    series_id: i64,
) -> Result<Vec<RawEpisode>, String> {
    let base = cfg.base.trim_end_matches('/');
    let mut url = player_api_url(cfg)?;
    url.query_pairs_mut()
        .append_pair("action", "get_series_info")
        .append_pair("series_id", &series_id.to_string());
    let json = get_json(client, url).await?;
    let parsed = parse_series_episodes(&json)?;
    let out = parsed
        .into_iter()
        .map(|p| RawEpisode {
            season: p.season,
            episode_num: p.episode_num,
            title: p.title,
            url: media_url(
                base,
                "series",
                &cfg.username,
                &cfg.password,
                &format!("{series_id}/{}/{}", p.season, p.episode_num),
                &p.ext,
            ),
            logo_url: p.logo_url,
        })
        .collect();
    Ok(out)
}

/// An episode as parsed from `get_series_info`, before URL materialization.
pub struct ParsedEpisode {
    pub season: i64,
    pub episode_num: i64,
    pub title: String,
    pub ext: String,
    pub logo_url: Option<String>,
}

fn parse_episode(ep: &serde_json::Value, season: Option<i64>) -> Option<ParsedEpisode> {
    let episode_num = ep.get("episode_num").and_then(|v| v.as_i64())?;
    let season = season.or_else(|| ep.get("season").and_then(|v| v.as_i64()))?;
    let title = ep
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("S{season}E{episode_num}"));
    let ext = ep
        .get("container_extension")
        .and_then(|v| v.as_str())
        .filter(|e| !e.is_empty())
        .map(String::from)
        .unwrap_or_else(|| "mp4".to_string());
    let logo_url = ep
        .get("info")
        .and_then(|i| i.get("movie_image"))
        .and_then(|v| v.as_str())
        .filter(|l| !l.is_empty())
        .map(String::from);
    Some(ParsedEpisode { season, episode_num, title, ext, logo_url })
}

/// Parses a `get_series_info` response into episodes. Handles the standard
/// `seasons` array shape, a flat top-level `episodes` array, and a
/// non-standard `episodes` object keyed by season number; an explicit error
/// object is surfaced as an error. Pure so it is unit testable without
/// network access.
pub fn parse_series_episodes(json: &serde_json::Value) -> Result<Vec<ParsedEpisode>, String> {
    if let Some(err) = json.get("error").and_then(|e| e.as_str()).filter(|e| !e.is_empty()) {
        return Err(format!("server error: {err}"));
    }
    let mut out = Vec::new();
    if let Some(seasons) = json.get("seasons").and_then(|s| s.as_array()) {
        for season in seasons {
            let season_num = season
                .get("season_number")
                .or_else(|| season.get("season"))
                .and_then(|v| v.as_i64());
            let Some(season_num) = season_num else {
                continue;
            };
            if let Some(eps) = season.get("episodes").and_then(|e| e.as_array()) {
                for ep in eps {
                    if let Some(p) = parse_episode(ep, Some(season_num)) {
                        out.push(p);
                    }
                }
            }
        }
    } else if let Some(eps) = json.get("episodes").and_then(|e| e.as_array()) {
        for ep in eps {
            if let Some(p) = parse_episode(ep, None) {
                out.push(p);
            }
        }
    } else if let Some(map) = json.get("episodes").and_then(|e| e.as_object()) {
        // Non-standard shape: `episodes` as an object keyed by season
        // number ({"episodes": {"1": [...], "2": [...]}}).
        for (season_key, eps) in map {
            let season = season_key.parse::<i64>().ok();
            if let Some(eps) = eps.as_array() {
                for ep in eps {
                    if let Some(p) = parse_episode(ep, season) {
                        out.push(p);
                    }
                }
            }
        }
    }
    if out.is_empty() {
        // Diagnostic for panels with an unexpected shape: the caller sees
        // "No episodes." without this, so log the raw response (truncated).
        eprintln!(
            "[xtream] get_series_info returned no episodes; raw: {:.200}",
            json
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vod_and_series_urls() {
        let base = "https://panel.example";
        // Live (unchanged shape).
        assert_eq!(
            live_url(base, "u", "p", 42),
            "https://panel.example/live/u/p/42.ts"
        );
        // VOD with an explicit extension.
        assert_eq!(
            media_url(base, "movie", "u", "p", "42", "mkv"),
            "https://panel.example/movie/u/p/42.mkv"
        );
        // Series episode URL (season/episode path parts).
        assert_eq!(
            media_url(base, "series", "u", "p", "7/1/2", "mp4"),
            "https://panel.example/series/u/p/7/1/2.mp4"
        );
        // Unparseable base falls back to string concat.
        assert_eq!(
            media_url("not a url", "movie", "u", "p", "1", "mp4"),
            "not a url/movie/u/p/1.mp4"
        );
    }

    #[test]
    fn parse_series_episodes_fixture() {
        let json = json!({
            "info": { "name": "The Show" },
            "seasons": [
                {
                    "season_number": 1,
                    "episodes": [
                        {
                            "id": 100,
                            "episode_num": 1,
                            "title": "Pilot",
                            "container_extension": "mkv",
                            "info": { "movie_image": "https://img.example/s1e1.jpg" }
                        },
                        {
                            "id": 101,
                            "episode_num": 2,
                            "title": "",
                            "container_extension": null,
                            "info": null
                        }
                    ]
                },
                { "season_number": 2, "episodes": [] }
            ]
        });
        let eps = parse_series_episodes(&json).unwrap();
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].season, 1);
        assert_eq!(eps[0].episode_num, 1);
        assert_eq!(eps[0].title, "Pilot");
        assert_eq!(eps[0].ext, "mkv");
        assert_eq!(eps[0].logo_url.as_deref(), Some("https://img.example/s1e1.jpg"));
        // Missing title → S{n}E{n}; missing ext → mp4.
        assert_eq!(eps[1].title, "S1E2");
        assert_eq!(eps[1].ext, "mp4");
        assert!(eps[1].logo_url.is_none());
    }

    #[test]
    fn parse_series_episodes_flat_and_errors() {
        // Flat top-level episodes array (non-standard panels).
        let flat = json!({
            "episodes": [
                { "episode_num": 3, "season": 1, "title": "T", "container_extension": "mp4" }
            ]
        });
        let eps = parse_series_episodes(&flat).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].season, 1);

        // Missing season or episode_num → skipped.
        let sparse = json!({
            "episodes": [
                { "episode_num": 3, "title": "no season" },
                { "season": 1, "title": "no num" }
            ]
        });
        assert!(parse_series_episodes(&sparse).unwrap().is_empty());

        // Explicit error object → Err.
        let err = json!({ "error": "series not found" });
        assert!(parse_series_episodes(&err).is_err());

        // Empty object → empty, not an error.
        assert!(parse_series_episodes(&json!({})).unwrap().is_empty());
    }

    #[test]
    fn parse_series_episodes_object_keyed() {
        // Non-standard shape: `episodes` object keyed by season number.
        let map = json!({
            "episodes": {
                "1": [
                    { "episode_num": 1, "title": "One", "container_extension": "mp4" },
                    { "episode_num": 2, "title": "Two", "container_extension": "mp4" }
                ],
                "2": [
                    { "episode_num": 1, "title": "Three", "container_extension": "mp4" }
                ],
                "garbage": []
            }
        });
        let eps = parse_series_episodes(&map).unwrap();
        assert_eq!(eps.len(), 3);
        assert_eq!(eps[0].season, 1);
        assert_eq!(eps[0].episode_num, 1);
        assert_eq!(eps[2].season, 2);
    }
}
