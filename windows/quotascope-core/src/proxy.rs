//! Shared HTTP and helper-process proxy settings. URLs never enter reports.
use reqwest::blocking::Client;
use std::sync::{Mutex, OnceLock};
pub fn valid(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some()
            && u.fragment().is_none()
            && u.query().is_none()
            && (u.path().is_empty() || u.path() == "/")
    })
}
pub fn client(fallback: &Client) -> Client {
    let (mode, url) = crate::settings::with(|s| (s.proxy_mode.clone(), s.proxy_url.clone()));
    if mode == "system" {
        return fallback.clone();
    }
    static CACHE: OnceLock<Mutex<Option<(String, String, Client)>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Mutex::default).lock().unwrap();
    if let Some((old_mode, old_url, client)) = &*cache {
        if *old_mode == mode && *old_url == url {
            return client.clone();
        }
    }
    let builder = Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("QuotaScope/Windows")
        .no_proxy();
    let builder = if mode == "manual" && valid(&url) {
        match reqwest::Proxy::all(&url) {
            Ok(proxy) => builder.proxy(proxy),
            Err(_) => builder,
        }
    } else {
        builder
    };
    let client = builder.build().unwrap_or_else(|_| fallback.clone());
    *cache = Some((mode, url, client.clone()));
    client
}
pub fn environment(command: &mut std::process::Command) {
    let (mode, url) = crate::settings::with(|s| (s.proxy_mode.clone(), s.proxy_url.clone()));
    if mode == "system" {
        return;
    }
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        command.env_remove(key);
    }
    if mode == "manual" && valid(&url) {
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            command.env(key, &url);
        }
        command.env("NO_PROXY", "localhost,127.0.0.1,::1");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_addresses_reject_paths_and_non_http_schemes() {
        assert!(valid("http://127.0.0.1:7890"));
        assert!(valid("https://user:password@proxy.example:443"));
        assert!(!valid("file:///keys"));
        assert!(!valid("https://proxy.example/path?token=x"));
    }
}
