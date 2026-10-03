//! The console's own session, separately encrypted from an API key.
//! Transport keeps HTTP refusal distinct from authentication expiration.
use crate::deepseek_console::{self as protocol, Range};
use crate::http::HttpClient;
use crate::model::{AccountKey, Unavailability};
use serde_json::Value;
use std::io::Read;

pub fn secret(account: &AccountKey) -> String {
    format!("{}:console", account.id())
}

pub fn token(account: &AccountKey) -> Option<String> {
    crate::secrets::key_for(&secret(account)).filter(|s| !s.trim().is_empty())
}

pub fn set_token(account: &AccountKey, token: &str) -> bool {
    crate::secrets::set_key(&secret(account), token);
    let stored = self::token(account);
    if token.is_empty() {
        stored.is_none()
    } else {
        stored.as_deref() == Some(token)
    }
}

pub fn from_browser() -> Option<(String, String)> {
    let (values, browser) =
        crate::browser_storage::find(protocol::ORIGIN, &[protocol::STORAGE_KEY], |values| {
            protocol::storage_token(values).is_some()
        })?;
    Some((protocol::storage_token(&values)?, browser))
}

pub fn renew_from_browser(account: &AccountKey, refused: &str) -> Option<String> {
    // An additional account never borrows the primary browser session.
    // It can be changed only by that account's explicit import action.
    if !account.is_primary() {
        return None;
    }
    let (new, _) = from_browser()?;
    if new == refused || !set_token(account, &new) {
        return None;
    }
    Some(new)
}

#[derive(Clone, Copy)]
pub enum Route {
    Amount,
    Cost,
    Summary,
}

impl Route {
    fn path(self) -> &'static str {
        match self {
            Self::Amount => "/api/v0/usage/by_api_key/amount",
            Self::Cost => "/api/v0/usage/by_api_key/cost",
            Self::Summary => "/api/v0/users/get_user_summary",
        }
    }
}

pub fn status_problem(status: u16) -> Option<Unavailability> {
    match status {
        200 => None,
        401 => Some(Unavailability::SessionExpired),
        429 => Some(Unavailability::RateLimited),
        // In particular, 403 and redirects never trigger a credential swap.
        _ => Some(Unavailability::ServerError),
    }
}

pub fn get(
    http: &HttpClient,
    route: Route,
    range: Option<&Range>,
    token: &str,
) -> Result<Value, Unavailability> {
    if !crate::scan::checkpoint() {
        return Err(Unavailability::Loading);
    }
    let mut url = url::Url::parse(&format!("{}{}", protocol::ORIGIN, route.path()))
        .map_err(|_| Unavailability::UnreadableReply)?;
    if let Some(range) = range {
        url.query_pairs_mut().extend_pairs(range.query());
    }
    let response = http
        .client_for_login()
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .map_err(|_| Unavailability::Unreachable)?;
    if let Some(problem) = status_problem(response.status().as_u16()) {
        return Err(problem);
    }
    // Retain only a bounded response; never save the console's raw account
    // envelope, API key tracking identities or its token-estimation fields.
    const LIMIT: usize = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    response
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Unavailability::UnreadableReply)?;
    if !crate::scan::checkpoint() {
        return Err(Unavailability::Loading);
    }
    if bytes.len() > LIMIT {
        return Err(Unavailability::UnreadableReply);
    }
    serde_json::from_slice(&bytes).map_err(|_| Unavailability::UnreadableReply)
}

/// One different, successfully persisted session may repair an auth error.
/// Never call the renewal callback for service refusal, rate limits or bad JSON.
pub fn renewing<T>(
    old: &str,
    mut read: impl FnMut(&str) -> Result<T, Unavailability>,
    renew: impl FnOnce() -> Option<String>,
) -> (Result<T, Unavailability>, Option<String>) {
    let first = read(old);
    if !matches!(first, Err(Unavailability::SessionExpired)) || !crate::scan::checkpoint() {
        return (first, None);
    }
    let Some(new) = renew().filter(|new| new != old) else {
        return (first, None);
    };
    (read(&new), Some(new))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;
    use std::cell::Cell;
    #[test]
    fn console_slots_do_not_alias_api_keys_or_other_accounts() {
        let primary = AccountKey::primary(Provider::DeepSeek);
        let extra = AccountKey::from_id("deepSeek#1").unwrap();
        assert_eq!(secret(&primary), "deepSeek:console");
        assert_eq!(secret(&extra), "deepSeek#1:console");
        assert_ne!(secret(&primary), primary.id());
        assert_ne!(secret(&primary), secret(&extra));
    }
    #[test]
    fn http_403_and_redirects_are_not_expired_logins() {
        assert_eq!(status_problem(200), None);
        assert_eq!(status_problem(401), Some(Unavailability::SessionExpired));
        assert_eq!(status_problem(429), Some(Unavailability::RateLimited));
        for code in [204, 302, 403, 404, 500] {
            assert_eq!(status_problem(code), Some(Unavailability::ServerError));
        }
    }
    #[test]
    fn only_auth_errors_try_one_different_token() {
        let renewals = Cell::new(0);
        for reason in [
            Unavailability::RateLimited,
            Unavailability::ServerError,
            Unavailability::UnreadableReply,
            Unavailability::Unreachable,
        ] {
            let (answer, new) = renewing(
                "old",
                |_| Err::<(), _>(reason),
                || {
                    renewals.set(renewals.get() + 1);
                    Some("new".into())
                },
            );
            assert_eq!(answer, Err(reason));
            assert!(new.is_none());
        }
        assert_eq!(renewals.get(), 0);
        let mut reads = Vec::new();
        let (answer, new) = renewing(
            "old",
            |token| {
                reads.push(token.to_string());
                if token == "old" {
                    Err(Unavailability::SessionExpired)
                } else {
                    Ok(7)
                }
            },
            || Some("new".into()),
        );
        assert_eq!(answer, Ok(7));
        assert_eq!(new.as_deref(), Some("new"));
        assert_eq!(reads, ["old", "new"]);
        let mut reads = 0;
        let (answer, _) = renewing(
            "old",
            |_| {
                reads += 1;
                Err::<(), _>(Unavailability::SessionExpired)
            },
            || Some("old".into()),
        );
        assert!(answer.is_err());
        assert_eq!(reads, 1);
        reads = 0;
        let (answer, _) = renewing(
            "old",
            |_| {
                reads += 1;
                Err::<(), _>(Unavailability::SessionExpired)
            },
            || Some("new".into()),
        );
        assert!(answer.is_err());
        assert_eq!(reads, 2);
    }
}
