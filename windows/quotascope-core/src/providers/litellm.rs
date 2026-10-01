//! LiteLLM, a self-hosted proxy: the budget on the user a virtual key belongs
//! to, and the budget on its team, each as spend against a limit the proxy
//! states.
//!
//! Read with the virtual key and the proxy address the user enters, from
//! three of LiteLLM's own routes: `GET /key/info` names the key's user and
//! team, then `GET /user/info?user_id=…` — or, for a key with a team and no
//! user, `GET /team/info?team_id=…` — carries the spend and the budget. The
//! key goes nowhere but that address. The shapes are second-hand — taken
//! from CodexBar's LiteLLM provider and its tests, not from a captured
//! reply — and the fixtures in the tests say so.
//!
//! **The team's budget is drawn first**, because it is the one the proxy
//! enforces on the key. A user or team with no `max_budget` has spend and
//! nothing to measure it against, and is left off. The ids the proxy answers
//! with are checked against the ones `/key/info` named, so another user's or
//! another team's budget is never drawn as this key's.
//!
//! A deployment that will not let a key read its own information (403, 404)
//! has no budget readable here: CodexBar falls back to a month's spend
//! report there, which is spend with no limit, and there is nowhere to show
//! that.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::gateway;
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use std::sync::Arc;

/// The named lengths a duration can state: five hours, a day and a week are
/// their own kinds, and a day reads as a day even when the proxy wrote
/// `24h`.
const FIVE_HOURS: i64 = 5 * 3_600;
const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

pub struct LiteLlmService {
    http: Arc<HttpClient>,
}

impl LiteLlmService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// The identity route, with its status read by hand. A deployment that
    /// will not let a key read its own information answers 403 or 404, and
    /// that is a "no limits" rather than a refused key; `fetch_json` folds
    /// 403 into the refused credential, so this one call keeps the
    /// distinction upstream makes.
    fn key_info_json(&self, url: &str, key: &str) -> Result<serde_json::Value, Unavailability> {
        let response = self
            .http
            .client_for_login()
            .get(url)
            .header("Authorization", format!("Bearer {key}"))
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        if status == 403 || status == 404 {
            return Err(Unavailability::NoLimitsReported);
        }
        match status {
            200..=299 => {}
            // A redirect is never followed, and for this route the one a
            // gateway sends is to its sign-in page.
            300..=399 | 401 => return Err(Unavailability::ApiKeyRefused),
            429 => return Err(Unavailability::RateLimited),
            _ => return Err(Unavailability::ServerError),
        }
        response.json().map_err(|_| Unavailability::UnreadableReply)
    }
}

impl ProviderService for LiteLlmService {
    fn provider(&self) -> Provider {
        Provider::LiteLlm
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::LiteLlm);
        let Some(key) = pasted_or_none(keys.api_key(Provider::LiteLlm)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let Some(address) = keys
            .address(Provider::LiteLlm)
            .filter(|s| !s.trim().is_empty())
        else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressMissing);
        };
        let Some(key_info_url) = gateway::url_from(&address, "/key/info", &["/v1"]) else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        let reply = match self.key_info_json(&key_info_url, &key) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let Some(identity) = identity(&reply) else {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        };

        let (path, query) = if let Some(user) = &identity.user {
            ("/user/info", ("user_id", user))
        } else if let Some(team) = &identity.team {
            ("/team/info", ("team_id", team))
        } else {
            // A key bound to neither — a master key — has no budget of its own.
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        };
        let Some(mut route) = gateway::url_from(&address, path, &["/v1"]) else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        route.push('?');
        route.push_str(&format!("{}={}", urlencode(query.0), urlencode(query.1)));

        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".to_string()),
        ];
        let header_refs: Vec<(&str, &str)> =
            headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self
            .http
            .fetch_json(crate::http::Method::Get, &route, &header_refs, None)
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let read = if identity.user.is_some() {
            reading_user_info(&reply, &identity)
        } else {
            reading_team_info(&reply, &identity)
        };
        match read {
            Ok(list) => {
                let mut usage = ProviderUsage::live_now(account, list);
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// The user and team a key belongs to, as `/key/info` names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user: Option<String>,
    pub team: Option<String>,
}

/// A budget as LiteLLM keeps it on a user or a team. Decoded strictly: a
/// figure that arrived as a string fails the reply rather than reading as
/// zero, which would draw a spent limit out of nothing.
#[derive(Deserialize)]
struct Budget {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    spend: Option<f64>,
    #[serde(default)]
    max_budget: Option<f64>,
    #[serde(default)]
    budget_duration: Option<String>,
    #[serde(default)]
    budget_reset_at: Option<String>,
}

#[derive(Deserialize)]
struct KeyInfo {
    #[serde(default)]
    info: Option<KeyIdentity>,
}

#[derive(Deserialize)]
struct KeyIdentity {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
}

#[derive(Deserialize)]
struct UserInfo {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_info: Option<Budget>,
    #[serde(default)]
    teams: Option<Vec<Budget>>,
}

#[derive(Deserialize)]
struct TeamInfo {
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    team_info: Option<Budget>,
}

/// The user and team `/key/info` names, blank ones as none. None at all when
/// the reply carries no `info` to read.
pub fn identity(reply: &serde_json::Value) -> Option<Identity> {
    let info: KeyInfo = serde_json::from_value(reply.clone()).ok()?;
    let info = info.info?;
    let named = |text: Option<&str>| -> Option<String> {
        text.map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    Some(Identity {
        user: named(info.user_id.as_deref()),
        team: named(info.team_id.as_deref()),
    })
}

/// The budgets behind a key with a user: the team's first — it is the one
/// the proxy enforces — then the user's own. The ids the proxy answered with
/// are checked against the ones `/key/info` named, so another user's or
/// another team's budget is never drawn as this key's.
pub fn reading_user_info(
    reply: &serde_json::Value,
    identity: &Identity,
) -> Result<Vec<UsageWindow>, Unavailability> {
    let reply: UserInfo =
        serde_json::from_value(reply.clone()).map_err(|_| Unavailability::UnreadableReply)?;
    let Some(user) = reply.user_info else {
        return Err(Unavailability::UnreadableReply);
    };
    // Whichever id the proxy answered with has to be the one asked about.
    if let Some(answered) = user.user_id.as_deref().or(reply.user_id.as_deref()) {
        if Some(answered) != identity.user.as_deref() {
            return Err(Unavailability::UnreadableReply);
        }
    }

    let mut windows = Vec::new();
    if let Some(team) = &identity.team {
        if let Some(budget) = reply
            .teams
            .iter()
            .flatten()
            .find(|budget| budget.team_id.as_deref() == Some(team.as_str()))
            .and_then(|budget| window(budget, "litellm.team", true))
        {
            windows.push(budget);
        }
    }
    if let Some(window) = window(&user, "litellm.user", false) {
        windows.push(window);
    }
    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    Ok(windows)
}

/// The budget behind a key with a team and no user: the team's own.
pub fn reading_team_info(
    reply: &serde_json::Value,
    identity: &Identity,
) -> Result<Vec<UsageWindow>, Unavailability> {
    let reply: TeamInfo =
        serde_json::from_value(reply.clone()).map_err(|_| Unavailability::UnreadableReply)?;
    let Some(team) = reply.team_info else {
        return Err(Unavailability::UnreadableReply);
    };
    if let Some(answered) = team.team_id.as_deref().or(reply.team_id.as_deref()) {
        if Some(answered) != identity.team.as_deref() {
            return Err(Unavailability::UnreadableReply);
        }
    }
    let Some(window) = window(&team, "litellm.team", true) else {
        return Err(Unavailability::NoLimitsReported);
    };
    Ok(vec![window])
}

/// A budget's spend against its limit. Upstream kinds a team's pool
/// `.sharedCredits` so the two rows are told apart and never read as one;
/// the Windows kind set has no shared-credits case, so Spend stands in and
/// the id says which is which.
fn window(budget: &Budget, id: &str, team: bool) -> Option<UsageWindow> {
    let spend = budget.spend.filter(|v| v.is_finite() && *v >= 0.0)?;
    let limit = budget.max_budget.filter(|v| v.is_finite() && *v > 0.0)?;
    let period = period(budget.budget_duration.as_deref());
    let fraction = spend / limit;
    let mut window = UsageWindow::new(
        id,
        if team { Kind::Spend } else { period.kind },
        None,
        fraction,
        period.seconds,
        budget
            .budget_reset_at
            .as_deref()
            .and_then(crate::timeutil::parse_iso8601_ms),
    );
    window.reports_length = period.stated;
    window.is_exhausted = fraction >= 1.0;
    Some(window)
}

/// LiteLLM's `budget_duration`: a count and a unit — `30s`, `12h`, `7d`,
/// `1mo`. Seconds, minutes, hours, days and weeks are a stated length; a
/// month is a calendar month and only sorts. No duration is a budget that
/// never turns over.
pub struct Period {
    pub kind: Kind,
    pub seconds: i64,
    pub stated: bool,
}

pub fn period(duration: Option<&str>) -> Period {
    let unstated = Period {
        kind: Kind::Spend,
        seconds: 30 * 86_400,
        stated: false,
    };
    let Some(text) = duration
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase)
    else {
        return unstated;
    };
    let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
    let Ok(count) = digits.parse::<i64>() else {
        return unstated;
    };
    if count <= 0 {
        return unstated;
    }
    let seconds = match text[digits.len()..].as_ref() {
        // A month is not a fixed length: a name and a sort key only.
        "mo" => {
            return Period {
                kind: if count == 1 {
                    Kind::Monthly
                } else {
                    Kind::Spend
                },
                seconds: count * 30 * 86_400,
                stated: false,
            };
        }
        "s" => count,
        "m" => count * 60,
        "h" => count * 3_600,
        "d" => count * 86_400,
        "w" => count * 7 * 86_400,
        _ => return unstated,
    };
    // Upstream's `.daily` case has no Windows `Kind`; the day as `other` is
    // the stand-in the rest of the port already uses.
    let kind = match seconds {
        FIVE_HOURS => Kind::FiveHour,
        DAY => Kind::Other(DAY),
        WEEK => Kind::Weekly,
        other => Kind::Other(other),
    };
    Period {
        kind,
        seconds,
        stated: true,
    }
}

fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both() -> Identity {
        Identity {
            user: Some("user-123".to_string()),
            team: Some("team-456".to_string()),
        }
    }

    /// The shapes pinned by upstream's `litellm-key-info` and
    /// `litellm-user-info` fixtures — second-hand, written from CodexBar's
    /// LiteLLM provider, not captured from a live proxy.
    fn key_info() -> serde_json::Value {
        serde_json::json!({
            "key": "sk-redacted",
            "info": {
                "key_name": "sk-...IAAw",
                "spend": 212.3537162499998,
                "expires": "2026-09-11T00:12:55.950000+00:00",
                "user_id": "user-123",
                "team_id": "team-456",
                "max_budget": null
            }
        })
    }

    fn user_info() -> serde_json::Value {
        serde_json::json!({
            "user_id": "user-123",
            "user_info": {
                "user_id": "user-123",
                "user_alias": "litellm-user@example.com",
                "max_budget": 300.0,
                "spend": 212.3537162499998,
                "user_email": "litellm-user@example.com",
                "budget_reset_at": null,
                "teams": ["team-456"],
                "metadata": {"source": "keycloak", "budget": 300, "flags": {"keycloak": true}}
            },
            "keys": [{"key_name": "sk-...OTHER", "user_id": "user-123", "team_id": "team-other"}],
            "teams": [
                {"team_alias": "unrelated", "team_id": "team-other", "max_budget": 5.0, "spend": 4.0},
                {"team_alias": "ai", "team_id": "team-456", "max_budget": 1000.0, "spend": 215.3245658499998, "budget_duration": "7d", "budget_reset_at": "2026-06-15T00:00:00Z"}
            ]
        })
    }

    #[test]
    fn key_info_names_the_keys_user_and_team() {
        assert_eq!(identity(&key_info()), Some(both()));
        let blank = serde_json::json!({"info": {"user_id": "  ", "team_id": "team-1"}});
        assert_eq!(
            identity(&blank),
            Some(Identity {
                user: None,
                team: Some("team-1".to_string())
            })
        );
        // No `info` to read is no identity at all.
        assert_eq!(identity(&serde_json::json!({"key": "sk"})), None);
        assert_eq!(identity(&serde_json::json!([])), None);
    }

    #[test]
    fn the_keys_team_budget_first_then_the_users_another_teams_is_ignored() {
        let windows = reading_user_info(&user_info(), &both()).unwrap();
        assert_eq!(
            windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["litellm.team", "litellm.user"]
        );

        let team = &windows[0];
        // Upstream kinds this `.sharedCredits`; Spend is the Windows stand-in.
        assert_eq!(team.kind, Kind::Spend);
        assert!((team.used_fraction - 215.3245658499998 / 1_000.0).abs() < 0.000_001);
        // `budget_duration: 7d` is a stated length.
        assert_eq!(team.window_seconds, 7 * 86_400);
        assert!(team.reports_length);
        assert_eq!(
            team.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-06-15T00:00:00Z")
        );

        let user = &windows[1];
        assert!((user.used_fraction - 212.3537162499998 / 300.0).abs() < 0.000_001);
        // No duration: never turns over, no length claimed.
        assert!(!user.reports_length);
        assert_eq!(user.resets_at, None);
    }

    #[test]
    fn a_team_only_key_reads_the_teams_own_budget() {
        let reply = serde_json::json!({
            "team_id": "team-456",
            "team_info": {
                "team_id": "team-456", "team_alias": "ai",
                "spend": 25, "max_budget": 100,
                "budget_duration": "30d", "budget_reset_at": "2026-10-01T00:00:00Z"
            }
        });
        let identity = Identity {
            user: None,
            team: Some("team-456".to_string()),
        };
        let windows = reading_team_info(&reply, &identity).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "litellm.team");
        assert_eq!(windows[0].used_fraction, 0.25);
        assert_eq!(windows[0].window_seconds, 30 * 86_400);
        assert!(windows[0].reports_length);
    }

    #[test]
    fn spend_with_no_budget_is_no_limits_not_a_ring_at_zero() {
        let reply = serde_json::json!({
            "user_id": "user-123",
            "user_info": {"user_id": "user-123", "max_budget": null, "spend": 12.5}
        });
        let identity = Identity {
            user: Some("user-123".to_string()),
            team: None,
        };
        assert_eq!(
            reading_user_info(&reply, &identity),
            Err(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn an_answer_about_someone_else_cant_be_read() {
        let identity = Identity {
            user: Some("user-123".to_string()),
            team: None,
        };
        for reply in [
            serde_json::json!({"user_info": {"user_id": "other", "max_budget": 10, "spend": 1}}),
            serde_json::json!({"user_id": "other", "user_info": {"max_budget": 10, "spend": 1}}),
            // A figure that arrived as a string fails the reply rather than
            // reading as zero.
            serde_json::json!({"user_info": {"spend": "4"}}),
            serde_json::json!({"teams": []}),
        ] {
            assert_eq!(
                reading_user_info(&reply, &identity),
                Err(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn another_teams_answer_cant_be_read() {
        let reply = serde_json::json!({
            "team_info": {"team_id": "other", "spend": 25, "max_budget": 100}
        });
        let identity = Identity {
            user: None,
            team: Some("team-456".to_string()),
        };
        assert_eq!(
            reading_team_info(&reply, &identity),
            Err(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn budget_duration_becomes_a_stated_length_a_month_only_a_sort_key() {
        let cases: &[(&str, Kind, i64, bool)] = &[
            ("5h", Kind::FiveHour, 5 * 3_600, true),
            ("1d", Kind::Other(86_400), 86_400, true),
            ("24h", Kind::Other(86_400), 86_400, true),
            ("7d", Kind::Weekly, 7 * 86_400, true),
            ("2w", Kind::Other(14 * 86_400), 14 * 86_400, true),
            ("30d", Kind::Other(30 * 86_400), 30 * 86_400, true),
            ("1mo", Kind::Monthly, 30 * 86_400, false),
            ("soon", Kind::Spend, 30 * 86_400, false),
        ];
        for (text, kind, seconds, stated) in cases {
            let period = period(Some(text));
            assert_eq!(period.kind, *kind, "{text}");
            assert_eq!(period.seconds, *seconds, "{text}");
            assert_eq!(period.stated, *stated, "{text}");
        }
        // No duration at all is a budget that never turns over.
        let unstated = period(None);
        assert_eq!(unstated.kind, Kind::Spend);
        assert!(!unstated.stated);
    }

    #[test]
    fn the_v1_base_is_dropped_and_the_user_id_is_the_query() {
        // These routes sit beside the `/v1` base rather than under it.
        assert_eq!(
            gateway::url_from("https://litellm.example.com/v1", "/user/info", &["/v1"]).as_deref(),
            Some("https://litellm.example.com/user/info")
        );
        assert_eq!(urlencode("user-123"), "user-123");
        assert_eq!(urlencode("user@example.com"), "user%40example.com");
    }
}
