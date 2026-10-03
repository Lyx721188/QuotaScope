//! The console's own session, separately encrypted from an API key.
//! Transport keeps HTTP refusal distinct from authentication expiration.
use crate::deepseek_console::{self as protocol, Range};
use crate::http::HttpClient;
use crate::model::{AccountKey, Unavailability};
use serde_json::Value;
use std::io::Read;

static MUTATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn secret(account: &AccountKey) -> String {
    format!("{}:console", account.id())
}

pub fn token(account: &AccountKey) -> Option<String> {
    crate::secrets::key_for(&secret(account)).filter(|s| !s.trim().is_empty())
}

pub fn set_token(account: &AccountKey, token: &str) -> bool {
    let _write = MUTATION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    persist_token(account, token)
}

fn persist_token(account: &AccountKey, token: &str) -> bool {
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
    let _write = MUTATION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // A user may have cleared or replaced the login while browser scanning
    // was in flight. Renewal must never restore that old credential slot.
    if new == refused || token(account).as_deref() != Some(refused) || !persist_token(account, &new)
    {
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
    fetch(http.client_for_login(), url, token)
}

fn fetch(
    client: reqwest::blocking::Client,
    url: url::Url,
    token: &str,
) -> Result<Value, Unavailability> {
    let response = client
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
    fn raw_transport_sends_one_bearer_get_and_classifies_real_http_replies() {
        use std::io::{BufRead, Write};
        for (status, body, expected) in [
            (200, "{\"code\":0}", None),
            (200, "not json", Some(Unavailability::UnreadableReply)),
            (401, "{}", Some(Unavailability::SessionExpired)),
            (403, "{}", Some(Unavailability::ServerError)),
            (429, "{}", Some(Unavailability::RateLimited)),
            (302, "{}", Some(Unavailability::ServerError)),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/redirect-must-not-follow\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                request
            });
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap();
            let url = url::Url::parse(&format!("http://{address}/console-test?tz=28800")).unwrap();
            let answer = fetch(client, url, "synthetic-console-token");
            if let Some(reason) = expected {
                assert_eq!(answer.unwrap_err(), reason);
            } else {
                assert_eq!(answer.unwrap()["code"], 0);
            }
            let request = server.join().unwrap().to_ascii_lowercase();
            assert!(request.starts_with("get /console-test?tz=28800 http/1.1\r\n"));
            assert!(request.contains("authorization: bearer synthetic-console-token\r\n"));
            assert!(request.contains("accept: application/json\r\n"));
        }
    }
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
