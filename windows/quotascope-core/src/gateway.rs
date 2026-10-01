//! Where a self-hosted gateway lives, as the reader typed it.
//!
//! Ported from `GatewayAddress.swift`. **A trust boundary, not a
//! convenience.** Every other provider in the directory knows where to go;
//! these are *told*, which means QuotaScope can be pointed at any host on
//! the internet with a credential attached. Whatever comes back from here
//! gets a bearer token put on it, so the checks below are about where that
//! token may go rather than about whether a string parses.
//!
//! Shared rather than copied per provider. Several gateways ask the same
//! question and differ only in the path they end at, and a second copy of a
//! rule like this is a second copy to forget to tighten.

use url::Url;

/// A checked URL for `path` on the gateway the reader named, or None if the
/// address may not be used.
///
/// - A scheme is assumed when none is typed, because `gateway.example.com`
///   is what people paste. It is assumed **https**, never http.
/// - Plain http only where there is nothing between QuotaScope and the
///   server to intercept it: loopback, a private network, or `.local`. On a
///   public host it is **refused rather than upgraded**, because silently
///   rewriting somebody's address is how a credential ends up somewhere
///   they never looked.
/// - No user info and no fragment. Both are ways of writing a URL whose
///   host is not the part a reader's eye lands on.
/// - No query either: the path built here is the whole request, and a query
///   pasted out of a dashboard link would be forwarded with the key.
///
/// `trimming` holds suffixes to drop off whatever the reader typed before
/// the route is appended. People paste a gateway's root and people paste
/// the base URL out of their client's config, which usually ends in `/v1`;
/// appending blindly makes `/v1/v1/…`.
pub fn url_from(typed: &str, path: &str, trimming: &[&str]) -> Option<String> {
    let trimmed = typed.trim();
    if trimmed.is_empty() {
        return None;
    }

    let text = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let mut parts = Url::parse(&text).ok()?;

    let scheme = parts.scheme().to_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let host = parts.host_str()?.to_string();
    if host.is_empty() {
        return None;
    }
    if !parts.username().is_empty() || parts.password().is_some() {
        return None;
    }
    if parts.fragment().is_some() || parts.query().is_some() {
        return None;
    }
    if scheme != "https" && !allows_plain_http(&host) {
        return None;
    }

    let root = root_of(parts.path(), trimming);
    parts.set_path(&format!("{root}{path}"));
    Some(parts.into())
}

/// Whether an address could be used at all, without naming a route.
///
/// What Settings checks on Save, so a reader is told the address is wrong
/// once rather than per endpoint.
pub fn is_usable(typed: &str) -> bool {
    url_from(typed, "/", &[]).is_some()
}

/// The gateway's root: trailing slashes gone, and any suffix the reader
/// already typed that the route is about to repeat.
///
/// Longest suffix first, so `/v1/usage` is not left as `/usage` by a rule
/// meant to strip `/v1`.
pub fn root_of(typed: &str, trimming: &[&str]) -> String {
    let mut path = typed.to_string();
    while path.ends_with('/') {
        path.pop();
    }

    let mut suffixes: Vec<&str> = trimming.to_vec();
    suffixes.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for suffix in suffixes {
        if path.ends_with(suffix) {
            for _ in 0..suffix.len() {
                path.pop();
            }
            while path.ends_with('/') {
                path.pop();
            }
            break;
        }
    }
    path
}

/// Whether http is safe for this host because nothing routable sits between
/// here and it.
///
/// Loopback, the three RFC 1918 ranges, IPv4 link-local, IPv6 loopback,
/// unique-local and link-local, and the `.local` names mDNS hands out.
/// Demanding a certificate on those would rule out the ordinary way people
/// run a gateway at home.
pub fn allows_plain_http(host: &str) -> bool {
    let name = host
        .to_lowercase()
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();

    if name == "localhost" || name.ends_with(".localhost") {
        return true;
    }
    if name == "::1" {
        return true;
    }
    if name.ends_with(".local") {
        return true;
    }
    // Unique-local (fc00::/7) and link-local (fe80::/10). The colon test is
    // what keeps a *name* beginning "fd" out of this.
    if name.contains(':')
        && (name.starts_with("fc") || name.starts_with("fd") || name.starts_with("fe80:"))
    {
        return true;
    }

    let octets: Vec<&str> = name.split('.').collect();
    if octets.len() != 4 {
        return false;
    }
    let numbers: Vec<u8> = octets.iter().filter_map(|o| o.parse::<u8>().ok()).collect();
    if numbers.len() != 4 {
        return false;
    }

    matches!(
        (numbers[0], numbers[1]),
        (127, _) | (10, _) | (192, 168) | (169, 254) | (172, 16..=31)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_becomes_https() {
        assert_eq!(
            url_from("gateway.example.com", "/v1/usage", &[]).as_deref(),
            Some("https://gateway.example.com/v1/usage")
        );
    }

    #[test]
    fn a_pasted_v1_root_is_not_doubled() {
        assert_eq!(
            url_from("https://host:8080/v1/", "/v1/usage", &["/v1"]).as_deref(),
            Some("https://host:8080/v1/usage")
        );
    }

    #[test]
    fn the_longest_suffix_wins() {
        assert_eq!(root_of("/v1/usage", &["/usage", "/v1"]), "/v1");
        assert_eq!(root_of("/v1/", &["/v1"]), "");
    }

    #[test]
    fn credentials_fragments_and_queries_are_refused() {
        assert!(url_from("https://user:key@host.example.com", "/", &[]).is_none());
        assert!(url_from("https://host.example.com/page#frag", "/", &[]).is_none());
        assert!(url_from("https://host.example.com/?token=1", "/", &[]).is_none());
        assert!(url_from("ftp://host.example.com", "/", &[]).is_none());
        assert!(url_from("", "/", &[]).is_none());
    }

    #[test]
    fn plain_http_is_loopback_private_and_local_only() {
        assert!(url_from("http://localhost:3000", "/", &[]).is_some());
        assert!(url_from("http://192.168.1.10:3000", "/", &[]).is_some());
        assert!(url_from("http://10.0.0.5", "/", &[]).is_some());
        assert!(url_from("http://mybox.local", "/", &[]).is_some());
        assert!(url_from("http://[::1]:8080", "/", &[]).is_some());
        // A public host is refused, not upgraded.
        assert!(url_from("http://gateway.example.com", "/", &[]).is_none());
        assert!(url_from("http://8.8.8.8", "/", &[]).is_none());
        // A *name* that begins like a unique-local address is still a name.
        assert!(!allows_plain_http("fdcorp.example.com"));
    }

    #[test]
    fn usability_is_the_save_button_s_question() {
        assert!(is_usable("gateway.example.com"));
        assert!(is_usable("http://localhost:4000"));
        assert!(!is_usable("https://user@host.example.com"));
    }
}
