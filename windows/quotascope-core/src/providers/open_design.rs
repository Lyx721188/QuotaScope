//! Open Design / OpenDesign Go. Protocol ported from Pulse 338c796.
//! Browser sign-in first, then the desktop prod login. Model API keys cannot
//! read billing. Login files are read only; credentials never follow redirects.
use super::{KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, Method};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, State, Unavailability, UsageWindow,
};
use serde_json::Value;
use std::io::Read;
use std::sync::Arc;

pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["open-design.ai", "amr-api.open-design.ai"],
    cookies: &[
        "__Secure-better-auth.session_token",
        "__Secure-better-auth.*",
    ],
};
const API: &str = "https://amr-api.open-design.ai";
const SESSION_TOKEN: &str = "__Secure-better-auth.session_token";

pub struct OpenDesignService {
    http: Arc<HttpClient>,
}

impl OpenDesignService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    fn fetch_for(&self, keys: &KeyRing, account: &AccountKey) -> ProviderUsage {
        // An added account must never borrow the desktop's primary login.
        let login = account
            .is_primary()
            .then(|| saved_login(&crate::home_dir()))
            .flatten();
        let cookie = keys
            .api_key(Provider::OpenDesign)
            .and_then(|s| normalize(&s));
        let read = |credential: Credential| {
            read_with(account, &credential, |url, workspace| {
                let mut headers = vec![("Accept", "application/json".to_string())];
                match &credential {
                    Credential::Session(cookie) => headers.extend([
                        ("Cookie", cookie.clone()),
                        ("Origin", "https://open-design.ai".into()),
                        ("Referer", "https://open-design.ai/cloud/dashboard".into()),
                    ]),
                    Credential::App(login) => {
                        headers.push(("Authorization", format!("Bearer {}", login.key)))
                    }
                }
                if let Some(id) = workspace {
                    headers.push(("x-vela-workspace-id", id.into()));
                }
                let refs: Vec<_> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
                self.http
                    .fetch_json(Method::Get, url, &refs, None)
                    .map_err(|reason| {
                        if reason == Unavailability::ApiKeyRefused {
                            credential.refused()
                        } else {
                            reason
                        }
                    })
            })
        };
        if let Some(cookie) = cookie {
            let usage = read(Credential::Session(cookie));
            if usage.state != State::Unavailable(Unavailability::SessionExpired) || login.is_none()
            {
                return usage;
            }
        }
        login
            .map(|login| read(Credential::App(login)))
            .unwrap_or_else(|| {
                ProviderUsage::unavailable(account.clone(), Unavailability::SessionMissing)
            })
    }
}

impl ProviderService for OpenDesignService {
    fn provider(&self) -> Provider {
        Provider::OpenDesign
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }
    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        self.fetch_for(keys, &AccountKey::primary(self.provider()))
    }
    fn fetch_account(&self, keys: &KeyRing, account: &AccountKey) -> Option<ProviderUsage> {
        Some(self.fetch_for(keys, account))
    }
}

#[derive(Clone)]
struct Login {
    key: String,
    api: url::Url,
}
enum Credential {
    Session(String),
    App(Login),
}
impl Credential {
    fn api(&self) -> url::Url {
        match self {
            Self::Session(_) => url::Url::parse(API).unwrap(),
            Self::App(l) => l.api.clone(),
        }
    }
    fn refused(&self) -> Unavailability {
        match self {
            Self::Session(_) => Unavailability::SessionExpired,
            Self::App(_) => Unavailability::LocalLoginExpired,
        }
    }
    fn routes(&self) -> &'static [&'static str] {
        match self {
            Self::Session(_) => &["coding-plan-usage", "snapshot"],
            Self::App(_) => &["funding-check", "coding-plan-usage", "snapshot"],
        }
    }
}

fn saved_login(home: &std::path::Path) -> Option<Login> {
    let mut bytes = Vec::new();
    std::fs::File::open(home.join(".amr/config.json"))
        .ok()?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    login_from(&serde_json::from_slice::<Value>(&bytes).ok()?)
}

fn login_from(root: &Value) -> Option<Login> {
    let prod = root.get("profiles")?.get("prod")?;
    if prod.get("pendingLogin").is_some_and(|v| !v.is_null()) {
        return None;
    }
    let key = prod.get("controlKey")?.as_str()?.trim();
    if key.is_empty() || key.bytes().any(|b| b < 32 || b == 127) {
        return None;
    }
    let api = prod
        .get("apiUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(API);
    let api = url::Url::parse(api).ok()?;
    let host = api.host_str()?;
    if api.scheme() != "https"
        || !api.username().is_empty()
        || api.password().is_some()
        || !(host == "open-design.ai" || host.ends_with(".open-design.ai"))
        || api.query().is_some()
        || api.fragment().is_some()
    {
        return None;
    }
    Some(Login {
        key: key.into(),
        api,
    })
}

pub fn normalize(input: &str) -> Option<String> {
    if input.bytes().any(|b| !(32..=126).contains(&b)) {
        return None;
    }
    let input = input.trim();
    let input = if input
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("cookie:"))
    {
        input[7..].trim()
    } else {
        input
    };
    let mut names = std::collections::HashSet::new();
    let mut kept = Vec::new();
    for pair in input.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        if !name.starts_with("__Secure-better-auth.") {
            continue;
        }
        if value.is_empty() || value.bytes().any(|b| b == b' ' || b == b'"' || b == b'\\') {
            return None;
        }
        if names.insert(name) {
            kept.push(format!("{name}={value}"));
        }
    }
    names.contains(SESSION_TOKEN).then(|| kept.join("; "))
}

fn read_with(
    account: &AccountKey,
    credential: &Credential,
    mut get: impl FnMut(&str, Option<&str>) -> Result<Value, Unavailability>,
) -> ProviderUsage {
    let mut base = credential.api();
    // Append path components, never interpret a workspace id as a URL/path.
    base.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .extend(["api", "v1", "workspaces"]);
    let root = match get(base.as_str(), None) {
        Ok(v) => v,
        Err(r) => return ProviderUsage::unavailable(account.clone(), r),
    };
    let personal: Vec<_> = root
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| v.get("workspaceType").and_then(Value::as_str) == Some("personal"))
        .collect();
    let chosen = personal
        .iter()
        .find(|v| v.get("isDefault").and_then(Value::as_bool) == Some(true))
        .or_else(|| personal.first());
    let Some(id) = chosen
        .and_then(|v| v.get("workspaceId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return ProviderUsage::unavailable(account.clone(), Unavailability::NoPlan);
    };
    let mut fallback = None;
    let mut last = Unavailability::NoLimitsReported;
    let mut refusals = 0;
    for route in credential.routes() {
        let mut url = base.clone();
        url.path_segments_mut()
            .unwrap()
            .extend([id, "billing", route]);
        match get(url.as_str(), Some(id)) {
            Err(r) if r == credential.refused() => refusals += 1,
            Err(r) => last = r,
            Ok(root) => {
                let usage = reading(&root, account, route == &"snapshot");
                if usage.state != State::Live {
                    continue;
                }
                if !usage.windows.is_empty() {
                    return usage;
                }
                if fallback.is_none() {
                    fallback = Some(usage);
                }
            }
        }
    }
    fallback.unwrap_or_else(|| {
        ProviderUsage::unavailable(
            account.clone(),
            if refusals == credential.routes().len() {
                Unavailability::NoPlan
            } else {
                last
            },
        )
    })
}

fn credits(value: &Value) -> Option<f64> {
    crate::http::number(value).filter(|v| v.is_finite() && *v >= 0.0)
}

pub fn reading(root: &Value, account: &AccountKey, snapshot: bool) -> ProviderUsage {
    if !root.is_object() {
        return ProviderUsage::unavailable(account.clone(), Unavailability::UnreadableReply);
    }
    let plan = if snapshot {
        root
    } else {
        root.get("codingPlan")
            .filter(|v| v.is_object())
            .unwrap_or(root)
    };
    let rows = plan
        .get(if snapshot {
            "designPlanWindows"
        } else {
            "windows"
        })
        .and_then(Value::as_array);
    let mut windows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows.into_iter().flatten() {
        let Some(limit) = row
            .get("limitCredits")
            .and_then(credits)
            .filter(|v| *v > 0.0)
        else {
            continue;
        };
        let remaining = row.get("remainingCredits").and_then(credits);
        let used = if snapshot {
            remaining.map(|r| (limit - r.min(limit)).max(0.0))
        } else {
            row.get("usedCredits")
                .and_then(credits)
                .or_else(|| remaining.map(|r| (limit - r.min(limit)).max(0.0)))
        };
        let Some(used) = used else {
            continue;
        };
        let seconds = if snapshot {
            match row.get("kind").and_then(Value::as_str) {
                Some("5h") => 18000,
                Some("7d") => 604800,
                _ => continue,
            }
        } else {
            let Some(seconds) = row
                .get("durationSeconds")
                .and_then(crate::http::number)
                .filter(|v| v.is_finite() && v.fract() == 0.0 && *v > 0.0 && *v <= 31.0 * 86400.0)
            else {
                continue;
            };
            seconds as i64
        };
        let kind = match seconds {
            18000 => Kind::FiveHour,
            86400 => Kind::Daily,
            604800 => Kind::Weekly,
            2419200..=2678400 => Kind::Monthly,
            _ => continue,
        };
        let id = if snapshot {
            format!(
                "openDesign.{}",
                if seconds == 18000 {
                    "fiveHour"
                } else {
                    "weekly"
                }
            )
        } else {
            format!(
                "openDesign.codingPlan.{}",
                row.get("policyId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{seconds}s"))
            )
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let mut window = UsageWindow::new(
            &id,
            kind,
            None,
            (used / limit).min(1.0),
            seconds,
            row.get("resetsAt")
                .and_then(Value::as_str)
                .and_then(crate::timeutil::parse_iso8601_ms),
        );
        window.is_exhausted = remaining.map(|r| r <= 0.0).unwrap_or(used >= limit);
        windows.push(window);
    }
    windows.sort_by_key(|w| w.window_seconds);
    let balance = if snapshot {
        root.get("wallet").and_then(|v| v.get("balanceUsd"))
    } else {
        root.get("balanceUsd")
    }
    .and_then(crate::http::number)
    .filter(|v| v.is_finite());
    if windows.is_empty() && balance.is_none() {
        return ProviderUsage::unavailable(account.clone(), Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(account.clone(), windows);
    usage.origin = Some("endpoint".into());
    usage.plan = (if snapshot {
        root.get("billing").and_then(|v| v.get("planId"))
    } else {
        plan.get("tier")
    })
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(|s| {
        let mut chars = s.chars();
        format!("{}{}", chars.next().unwrap().to_uppercase(), chars.as_str())
    });
    usage.credit_balance = balance.map(|v| format!("${v:.2}"));
    usage.credit_remaining = balance.map(|amount| CreditAmount {
        amount,
        currency: "USD".into(),
    });
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn account() -> AccountKey {
        AccountKey::primary(Provider::OpenDesign)
    }
    #[test]
    fn cookies_are_filtered_and_require_the_login() {
        assert_eq!(normalize("Cookie: analytics=no; __Secure-better-auth.session_token=test; __Secure-better-auth.other=yes"), Some("__Secure-better-auth.session_token=test; __Secure-better-auth.other=yes".into()));
        for s in [
            "sk-model-key",
            "__Secure-better-auth.other=yes",
            "__Secure-better-auth.session_token=",
            "__Secure-better-auth.session_token=a\r\nX: y",
        ] {
            assert!(normalize(s).is_none());
        }
    }
    #[test]
    fn desktop_login_trusts_only_completed_prod_logins_on_the_service() {
        for api in [
            "http://amr-api.open-design.ai",
            "https://open-design.ai.evil.test",
            "https://user@open-design.ai",
            "https://evil.test",
            "https://open-design.ai?token=x",
        ] {
            assert!(
                login_from(&json!({"profiles":{"prod":{"controlKey":"test","apiUrl":api}}}))
                    .is_none()
            );
        }
        assert!(
            login_from(&json!({"profiles":{"prod":{"controlKey":"test","pendingLogin":{}}}}))
                .is_none()
        );
        assert_eq!(
            login_from(&json!({"profiles":{"prod":{"controlKey":"test"}}}))
                .unwrap()
                .api
                .as_str(),
            format!("{API}/")
        );
    }
    #[test]
    fn go_windows_and_funding_check_wallet_use_reported_denominators() {
        let usage = reading(
            &json!({"balanceUsd":"2.45", "codingPlan":{"tier":"go","windows":[
                {"policyId":"week","durationSeconds":604800,"usedCredits":"30","limitCredits":"100","remainingCredits":"70"},
                {"policyId":"short","durationSeconds":18000,"limitCredits":"20","remainingCredits":"5","resetsAt":"2026-11-13T00:00:00Z"}
            ]}}),
            &account(),
            false,
        );
        assert_eq!(usage.state, State::Live);
        assert_eq!(usage.plan.as_deref(), Some("Go"));
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].used_fraction, 0.75);
        assert_eq!(usage.windows[1].used_fraction, 0.3);
        assert_eq!(usage.credit_remaining.unwrap().amount, 2.45);
    }
    #[test]
    fn snapshot_skips_unknown_missing_negative_and_zero_limits() {
        let usage = reading(
            &json!({"billing":{"planId":"pro"},"designPlanWindows":[
                {"kind":"7d","limitCredits":"100","remainingCredits":"0"},
                {"kind":"5h","limitCredits":"0","remainingCredits":"0"},
                {"kind":"5h","limitCredits":"10","remainingCredits":"-1"},
                {"kind":"unknown","limitCredits":"10","remainingCredits":"1"},
                {"kind":"5h","limitCredits":"10"}
            ]}),
            &account(),
            true,
        );
        assert_eq!(usage.windows.len(), 1);
        assert!(usage.windows[0].is_exhausted);
        assert_eq!(usage.windows[0].kind, Kind::Weekly);
    }
    #[test]
    fn refused_billing_route_does_not_end_a_valid_login() {
        let credential = Credential::Session("test".into());
        let mut routes = Vec::new();
        let usage = read_with(&account(), &credential, |url, workspace| {
            let path = url::Url::parse(url).unwrap().path().to_owned();
            routes.push(path.clone());
            if workspace.is_none() {
                return Ok(
                    json!({"items":[{"workspaceId":"team","workspaceType":"team","isDefault":true},{"workspaceId":"personal","workspaceType":"personal"}]}),
                );
            }
            assert_eq!(workspace, Some("personal"));
            if path.ends_with("coding-plan-usage") {
                return Err(Unavailability::SessionExpired);
            }
            Ok(
                json!({"designPlanWindows":[{"kind":"7d","limitCredits":"100","remainingCredits":"70"}]}),
            )
        });
        assert_eq!(usage.state, State::Live);
        assert_eq!(routes.len(), 3);
        assert!(routes[2].ends_with("personal/billing/snapshot"));
    }
    #[test]
    fn all_billing_refusals_mean_no_plan_and_wallet_is_a_fallback() {
        let c = Credential::Session("test".into());
        for wallet in [false, true] {
            let usage = read_with(&account(), &c, |_, workspace| {
                if workspace.is_none() {
                    Ok(json!({"items":[{"workspaceId":"p","workspaceType":"personal"}]}))
                } else if wallet {
                    Ok(json!({"wallet":{"balanceUsd":"0"}}))
                } else {
                    Err(Unavailability::SessionExpired)
                }
            });
            assert_eq!(
                usage.state,
                if wallet {
                    State::Live
                } else {
                    State::Unavailable(Unavailability::NoPlan)
                }
            );
        }
    }
}
