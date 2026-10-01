//! IBM Bob: the month's Bobcoins against each team's budget.
//!
//! Read with a key the user pastes, in two steps, from the admin routes Bob's
//! own tools call:
//! 1. `GET https://api.us-east.bob.ibm.com/admin/v1/profile` lists the
//!    subscriptions the key can see, each with its teams, the user's id in
//!    it, and the regional host that serves it.
//! 2. `GET https://<region>/admin/v1/teams/<team>/users/<user>` for each team
//!    gives the Bobcoins used and the budget.
//!
//! The shape is second-hand — taken from CodexBar's IBM Bob provider and its
//! tests, not from a captured reply.
//!
//! **The key goes to IBM Bob's hosts and nowhere else.** The regional host
//! comes from the reply; one that is not under `bob.ibm.com`, or that carries
//! anything but a bare host, is refused before anything is sent.
//!
//! **One ring, only when every team has a budget.** Bobcoins are counted per
//! month; a team with no budget is unlimited, and adding its usage to the
//! others' budgets would make a fraction nobody reported. So the sum is drawn
//! only when every team states one. The period is a billing month, so its
//! length is a sort key and is not claimed.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number, number_field, HttpClient};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const HOME: &str = "https://api.us-east.bob.ibm.com";

pub struct IbmBobService {
    http: Arc<HttpClient>,
}

impl IbmBobService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for IbmBobService {
    fn provider(&self) -> Provider {
        Provider::IbmBob
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::IbmBob);
        let Some(key) = pasted_or_none(keys.api_key(Provider::IbmBob)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let authorization = authorization(&key);
        let headers = [
            ("Authorization", authorization.clone()),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let reply = match self.http.fetch_json(
            crate::http::Method::Get,
            &format!("{HOME}/admin/v1/profile"),
            &refs,
            None,
        ) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let Some(profile) = decode_profile(&reply) else {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        };

        let mut teams: Vec<Team> = Vec::new();
        for instance in &profile.instances {
            let Some(user) = instance.user_id.as_deref().filter(|user| !user.is_empty()) else {
                continue;
            };
            // A region the key must not be sent to means the reply cannot be
            // trusted, not just that one team is missing.
            let Some(base) = regional_host(instance.region_domain.as_deref()) else {
                return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
            };
            for team in &instance.teams {
                if team.id.is_empty() {
                    continue;
                }
                let Some(url) = team_url(&base, &team.id, user) else {
                    return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
                };
                let team_headers = [
                    ("Authorization", authorization.clone()),
                    ("Accept", "application/json".into()),
                    ("x-instance-id", instance.instance_id.clone()),
                    ("x-team-id", team.id.clone()),
                ];
                let team_refs: Vec<(&str, &str)> =
                    team_headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let reply =
                    match self
                        .http
                        .fetch_json(crate::http::Method::Get, &url, &team_refs, None)
                    {
                        Ok(value) => value,
                        Err(reason) => return ProviderUsage::unavailable(account, reason),
                    };
                let Some(budget) = decode_budget(&reply) else {
                    return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
                };
                teams.push(Team {
                    used: budget.usage,
                    // The team's own budget wins; the profile's is the
                    // fallback for one the team route does not restate.
                    budget: budget.budget_limit.or(team.budget_limit),
                    plan: instance.plan_name.clone(),
                    resets_at: instance.refresh_at,
                });
            }
        }
        match reading(&teams) {
            Ok(reading) => {
                let mut usage = ProviderUsage::live_now(account, vec![reading.window]);
                usage.plan = reading.plan;
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// What the key is sent as: a JWT from a Bob sign-in as a bearer token,
/// anything else as an IBM API key.
pub fn authorization(key: &str) -> String {
    if is_jwt(key) {
        format!("Bearer {key}")
    } else {
        format!("Apikey {key}")
    }
}

/// A three-segment token whose middle segment decodes to a JSON object — the
/// shape of every JWT, and of nothing an IBM API key looks like.
pub fn is_jwt(token: &str) -> bool {
    use base64::Engine;
    // Empty segments count, the same way Swift's split with empty
    // subsequences kept does: "a..b" is three parts.
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    let mut payload = parts[1].replace('-', "+").replace('_', "/");
    payload.push_str(&"=".repeat((4 - payload.len() % 4) % 4));
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(payload.as_bytes()) else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&decoded)
        .ok()
        .is_some_and(|claims| claims.is_object())
}

/// The regional host a subscription names, or `None` for one the key must not
/// be sent to. No region named is the home host.
pub fn regional_host(domain: Option<&str>) -> Option<String> {
    let Some(domain) = domain.map(str::trim).filter(|domain| !domain.is_empty()) else {
        return Some(HOME.to_string());
    };
    let lowered = domain.to_lowercase();
    let host = if lowered.starts_with("api.") {
        lowered
    } else {
        format!("api.{lowered}")
    };
    // A bare host and nothing else: no path, port, user or query to hide
    // another host behind.
    let plain = host
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    if plain && (host == "bob.ibm.com" || host.ends_with(".bob.ibm.com")) {
        Some(format!("https://{host}"))
    } else {
        None
    }
}

/// One team's month, as the per-team route reported it.
pub struct Team {
    pub used: f64,
    pub budget: Option<f64>,
    pub plan: Option<String>,
    pub resets_at: Option<i64>,
}

/// What one reply yields, kept apart from the HTTP plumbing so tests can feed
/// it fixtures directly.
#[derive(Debug)]
pub struct BobReading {
    pub window: UsageWindow,
    pub plan: Option<String>,
}

pub fn reading(teams: &[Team]) -> Result<BobReading, Unavailability> {
    if teams.is_empty() {
        return Err(Unavailability::NoPlan);
    }
    let plans: std::collections::BTreeSet<String> = teams
        .iter()
        .filter_map(|team| {
            team.plan
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
        })
        .map(str::to_string)
        .collect();
    let plan = if plans.is_empty() {
        None
    } else {
        Some(plans.into_iter().collect::<Vec<_>>().join(", "))
    };

    let budgets: Vec<f64> = teams.iter().filter_map(|team| team.budget).collect();
    if budgets.len() != teams.len()
        || !teams
            .iter()
            .all(|team| team.used.is_finite() && team.used >= 0.0)
        || !budgets
            .iter()
            .all(|budget| budget.is_finite() && *budget >= 0.0)
    {
        // A team with no budget is unlimited; summing its usage into the
        // others' budgets would draw a fraction nobody reported.
        return Err(Unavailability::NoLimitsReported);
    }
    let used: f64 = teams.iter().map(|team| team.used).sum();
    let budget: f64 = budgets.iter().sum();
    if budget <= 0.0 {
        return Err(Unavailability::NoLimitsReported);
    }

    let mut window = UsageWindow::new(
        "ibmbob.bobcoins",
        Kind::Monthly,
        None,
        used / budget,
        // A sort key only: a billing month is not stated to be thirty days.
        30 * 86_400,
        teams.iter().filter_map(|team| team.resets_at).min(),
    );
    window.reports_length = false;
    window.is_exhausted = used >= budget;
    Ok(BobReading { window, plan })
}

/// The per-team route, with the ids percent-encoded as path segments — they
/// come from the reply, and a slash or `#` in one must not bend the path.
fn team_url(base: &str, team: &str, user: &str) -> Option<String> {
    let mut url = url::Url::parse(base).ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .push("admin")
        .push("v1")
        .push("teams")
        .push(team)
        .push("users")
        .push(user);
    Some(url.to_string())
}

// MARK: - Reading the replies

pub struct Profile {
    pub instances: Vec<Instance>,
}

pub struct Instance {
    pub instance_id: String,
    pub user_id: Option<String>,
    pub plan_name: Option<String>,
    /// Epoch milliseconds, when the profile names a refresh.
    pub refresh_at: Option<i64>,
    pub region_domain: Option<String>,
    pub teams: Vec<TeamRef>,
}

pub struct TeamRef {
    pub id: String,
    pub budget_limit: Option<f64>,
}

/// A field that may hold a string, a JSON null, or nothing at all — anything
/// else is a reply the whole decode refuses.
fn optional_string(reply: &serde_json::Value, key: &str) -> Option<Option<String>> {
    match reply.get(key) {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(serde_json::Value::String(text)) => Some(Some(text.clone())),
        Some(_) => None,
    }
}

fn optional_number(reply: &serde_json::Value, key: &str) -> Option<Option<f64>> {
    match reply.get(key) {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(raw) => Some(number(raw)),
    }
}

/// `refresh_at` has come as Unix seconds and as an ISO 8601 string, and a
/// value of either shape that says no date is simply no date.
fn moment(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(seconds) => seconds
            .as_f64()
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(crate::timeutil::epoch_to_ms),
        serde_json::Value::String(text) => crate::timeutil::parse_iso8601_ms(text),
        _ => None,
    }
}

pub fn decode_profile(reply: &serde_json::Value) -> Option<Profile> {
    let instances = reply.get("instances")?.as_array()?;
    let mut decoded = Vec::new();
    for instance in instances {
        let instance_id = instance.get("instance_id")?.as_str()?.to_string();
        let user_id = optional_string(instance, "user_id")?;
        let plan_name = optional_string(instance, "plan_name")?;
        let region_domain = optional_string(instance, "region_domain")?;
        let refresh_at = instance.get("refresh_at").and_then(moment);
        let teams_value = instance.get("teams")?.as_array()?;
        let mut teams = Vec::new();
        for team in teams_value {
            let id = team.get("id")?.as_str()?.to_string();
            let budget_limit = optional_number(team, "budget_limit")?;
            teams.push(TeamRef { id, budget_limit });
        }
        decoded.push(Instance {
            instance_id,
            user_id,
            plan_name,
            refresh_at,
            region_domain,
            teams,
        });
    }
    Some(Profile { instances: decoded })
}

pub struct Budget {
    pub usage: f64,
    pub budget_limit: Option<f64>,
}

pub fn decode_budget(reply: &serde_json::Value) -> Option<Budget> {
    Some(Budget {
        usage: number_field(reply, "usage")?,
        budget_limit: optional_number(reply, "budget_limit")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use serde_json::json;

    fn profile_reply() -> serde_json::Value {
        json!({
            "instances": [{
                "instance_id": "inst-1",
                "user_id": "user-9",
                "plan_name": "Bob Pro",
                "refresh_at": "2026-10-01T00:00:00Z",
                "region_domain": "api.eu-de.bob.ibm.com",
                "teams": [
                    { "id": "team-a", "budget_limit": 100.0 },
                    { "id": "team-b" },
                ],
            }],
        })
    }

    #[test]
    fn profile_decodes_teams_with_their_fallback_budgets() {
        let profile = decode_profile(&profile_reply()).unwrap();
        assert_eq!(profile.instances.len(), 1);
        let instance = &profile.instances[0];
        assert_eq!(instance.instance_id, "inst-1");
        assert_eq!(instance.teams.len(), 2);
        assert_eq!(instance.teams[0].budget_limit, Some(100.0));
        assert_eq!(instance.teams[1].budget_limit, None);
        assert!(instance.refresh_at.is_some());
    }

    #[test]
    fn a_wrongly_typed_field_refuses_the_whole_profile() {
        let mut bad = profile_reply();
        bad["instances"][0]["user_id"] = json!(5.0);
        assert!(decode_profile(&bad).is_none());
        let mut no_teams = profile_reply();
        no_teams["instances"][0]["teams"] = json!(null);
        assert!(decode_profile(&no_teams).is_none());
        assert!(decode_profile(&json!({ "nope": [] })).is_none());
    }

    #[test]
    fn refresh_at_reads_seconds_and_iso_strings_alike() {
        let seconds = decode_profile(&json!({ "instances": [{
            "instance_id": "i", "teams": [], "refresh_at": 1_791_168_000.0,
        }] }))
        .unwrap();
        assert_eq!(
            seconds.instances[0].refresh_at,
            Some(crate::timeutil::epoch_to_ms(1_791_168_000.0))
        );
        let nonsense = decode_profile(&json!({ "instances": [{
            "instance_id": "i", "teams": [], "refresh_at": { "x": 1 },
        }] }))
        .unwrap();
        assert_eq!(nonsense.instances[0].refresh_at, None);
    }

    #[test]
    fn budgets_sum_only_when_every_team_has_one() {
        let teams = vec![
            Team {
                used: 30.0,
                budget: Some(100.0),
                plan: Some("Bob Pro".into()),
                resets_at: None,
            },
            Team {
                used: 20.0,
                budget: Some(50.0),
                plan: Some("Bob Lite".into()),
                resets_at: None,
            },
        ];
        let reading = reading(&teams).unwrap();
        assert_eq!(reading.window.used_fraction, 50.0 / 150.0);
        assert_eq!(reading.plan.as_deref(), Some("Bob Lite, Bob Pro"));
        assert!(!reading.window.reports_length);
        assert_eq!(reading.window.kind, Kind::Monthly);
    }

    #[test]
    fn a_team_without_a_budget_is_unlimited_and_stops_the_ring() {
        let teams = vec![
            Team {
                used: 30.0,
                budget: Some(100.0),
                plan: None,
                resets_at: None,
            },
            Team {
                used: 999_999.0,
                budget: None,
                plan: None,
                resets_at: None,
            },
        ];
        assert_eq!(
            reading(&teams).unwrap_err(),
            Unavailability::NoLimitsReported
        );
        assert_eq!(reading(&[]).unwrap_err(), Unavailability::NoPlan);
    }

    #[test]
    fn the_ring_resets_when_the_soonest_team_resets() {
        let at = crate::timeutil::epoch_to_ms(1_791_168_000.0);
        let teams = vec![
            Team {
                used: 1.0,
                budget: Some(2.0),
                plan: None,
                resets_at: Some(at + 5_000),
            },
            Team {
                used: 1.0,
                budget: Some(2.0),
                plan: None,
                resets_at: Some(at),
            },
        ];
        let reading = reading(&teams).unwrap();
        assert_eq!(reading.window.resets_at, Some(at));
    }

    #[test]
    fn jwt_keys_ride_as_bearers_and_api_keys_as_apikeys() {
        let claims = serde_json::json!({ "sub": "user" }).to_string();
        let header = format!(
            "{}.{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("header"),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("sig")
        );
        assert!(is_jwt(&header));
        assert_eq!(authorization(&header), format!("Bearer {header}"));
        assert!(!is_jwt("a.b"));
        assert!(!is_jwt("not.a.jwt"));
        assert_eq!(authorization("abc123"), "Apikey abc123");
    }

    #[test]
    fn only_bob_hosts_receive_the_key() {
        assert_eq!(regional_host(None).as_deref(), Some(HOME));
        assert_eq!(regional_host(Some("  ")).as_deref(), Some(HOME));
        assert_eq!(
            regional_host(Some("eu-de.bob.ibm.com")).as_deref(),
            Some("https://api.eu-de.bob.ibm.com")
        );
        assert_eq!(
            regional_host(Some("API.US-East.Bob.IBM.Com")).as_deref(),
            Some("https://api.us-east.bob.ibm.com")
        );
        assert_eq!(
            regional_host(Some("bob.ibm.com")).as_deref(),
            Some("https://api.bob.ibm.com")
        );
        assert_eq!(regional_host(Some("evil.example.com")), None);
        assert_eq!(regional_host(Some("bob.ibm.com.evil.com")), None);
        assert_eq!(regional_host(Some("bob.ibm.com:8443")), None);
        assert_eq!(regional_host(Some("bob.ibm.com/admin")), None);
    }

    #[test]
    fn team_urls_encode_their_segments() {
        let url = team_url("https://api.eu-de.bob.ibm.com", "team/1", "user 2").unwrap();
        assert_eq!(
            url,
            "https://api.eu-de.bob.ibm.com/admin/v1/teams/team%2F1/users/user%202"
        );
    }

    #[test]
    fn budget_usage_is_required() {
        assert!(decode_budget(&json!({ "usage": 12.5, "budget_limit": 100.0 })).is_some());
        assert!(decode_budget(&json!({ "budget_limit": 100.0 })).is_none());
        assert!(decode_budget(&json!({ "usage": true })).is_none());
    }
}
