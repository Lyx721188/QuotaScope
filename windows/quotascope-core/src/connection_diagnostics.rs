//! User-initiated response capture. Thread-local, bounded and memory-only.
use serde_json::Value;
use std::cell::RefCell;

#[derive(Clone, Debug)]
pub struct Reply {
    pub method: String,
    pub address: String,
    pub status: Option<u16>,
    pub content_type: String,
    pub body: String,
}
thread_local! { static REPLIES: RefCell<Option<Vec<Reply>>> = const { RefCell::new(None) }; }
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<Reply>) {
    struct Restore(Option<Vec<Reply>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REPLIES.with(|r| *r.borrow_mut() = self.0.take());
        }
    }
    let previous = REPLIES.with(|r| r.replace(Some(Vec::new())));
    let _restore = Restore(previous);
    let result = f();
    let replies = REPLIES.with(|r| r.borrow_mut().take().unwrap_or_default());
    (result, replies)
}
fn sensitive(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "auth",
        "session",
        "jwt",
        "bearer",
        "token",
        "password",
        "secret",
        "cookie",
        "authorization",
        "credential",
        "apikey",
        "controlkey",
        "sessionkey",
        "accesskey",
        "csrf",
        "email",
    ]
    .iter()
    .any(|s| key.contains(s))
        || key == "key"
}
fn secrets(headers: &[(&str, &str)], body: Option<&Value>) -> Vec<String> {
    fn collect(value: &Value, private: bool, out: &mut Vec<String>) {
        match value {
            Value::String(s) if private && !s.is_empty() => out.push(s.clone()),
            Value::Object(obj) => {
                for (k, v) in obj {
                    collect(v, private || sensitive(k), out);
                }
            }
            Value::Array(items) => {
                for v in items {
                    collect(v, private, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (k, v) in headers {
        if sensitive(k) {
            out.push((*v).into());
            out.push(v.strip_prefix("Bearer ").unwrap_or(v).into());
            if k.eq_ignore_ascii_case("cookie") {
                for pair in v.split(';') {
                    if let Some((_, v)) = pair.split_once('=') {
                        if !v.trim().is_empty() {
                            out.push(v.trim().into());
                        }
                    }
                }
            }
        }
    }
    if let Some(body) = body {
        collect(body, false, &mut out);
    }
    for value in out.clone() {
        let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
        out.push(encoded.replace('+', "%20"));
        out.push(encoded);
        if let Ok(quoted) = serde_json::to_string(&value) {
            out.push(quoted[1..quoted.len() - 1].to_string());
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.len()));
    out.dedup();
    out
}
fn mask(text: &str, credentials: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in credentials {
        if !secret.is_empty() {
            text = text.replace(secret, "[masked]");
        }
    }
    use std::sync::OnceLock;
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(||regex::Regex::new(r"(?i)(?:bearer\s+[^\s\x22<>]+|(?:sk-|ghp_|github_pat_)[a-z0-9_\-]{8,}|\b[a-f0-9]{24,}\b|\beyJ[a-z0-9_\-]+\.[a-z0-9_\-]+(?:\.[a-z0-9_\-]+)?|[a-z0-9.!#$%&'*+/=?^_`{|}~\-]+@[a-z0-9.\-]+\.[a-z]{2,})").unwrap()).replace_all(&text,"[masked]").into_owned()
}
fn scrub(value: &mut Value, private: bool, credentials: &[String]) {
    match value {
        Value::String(s) => {
            *s = if private {
                "[masked]".into()
            } else {
                mask(s, credentials)
            }
        }
        Value::Object(obj) => {
            for (k, v) in obj {
                scrub(v, private || sensitive(k), credentials);
            }
        }
        Value::Array(items) => {
            for v in items {
                scrub(v, private, credentials);
            }
        }
        _ => {}
    }
}
pub fn record(
    method: &str,
    address: &str,
    status: Option<u16>,
    content_type: &str,
    body: &str,
    headers: &[(&str, &str)],
    request: Option<&Value>,
) {
    REPLIES.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(replies) = slot.as_mut().filter(|r| r.len() < 8) else { return; };
        let credentials = secrets(headers,request);
        let address = url::Url::parse(address).map(|mut u| {
            let _ = u.set_username(""); let _ = u.set_password(None); u.set_fragment(None);
            let keys: Vec<_> = u.query_pairs().map(|(k,_)|k.to_string()).collect();
            u.set_query(None);
            if !keys.is_empty() { let mut q=u.query_pairs_mut(); for k in keys { q.append_pair(&k,"[masked]"); } }
            mask(u.as_str(),&credentials)
        }).unwrap_or_else(|_| "[unreadable address]".into());
        let mut end = body.len().min(32768); while !body.is_char_boundary(end) { end-=1; }
        let mut rendered = if let Ok(mut value) = serde_json::from_str::<Value>(&body[..end]) { scrub(&mut value,false,&credentials); serde_json::to_string_pretty(&value).unwrap_or_default() } else {
            // Non-JSON payloads can carry credentials in script or markup.
            // Keep plain text only when no credential-like assignment remains.
            let text = mask(&body[..end],&credentials);
            static ASSIGN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            ASSIGN.get_or_init(||regex::Regex::new(r#"(?is)((?:token|secret|password|api[_-]?key|access[_-]?key|control[_-]?key|cookie|auth|session|credential|jwt)[\w-]*[\x22\s]*[:=]\s*)(?:\x22(?:\\.|[^\x22\\])*(?:\x22|$)|'(?:\\.|[^'\\])*(?:'|$)|[^\s\x22<>,;]+)"#).unwrap()).replace_all(&text,"${1}[masked]").into_owned()
        };
        if rendered.len() > 32768 { let mut end=32768; while !rendered.is_char_boundary(end) {end-=1;} rendered.truncate(end); rendered.push_str("\n[truncated]"); }
        if end < body.len() { rendered.push_str("\n[truncated]"); }
        replies.push(Reply { method:method.into(), address, status, content_type:content_type.chars().take(120).collect(), body:rendered });
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn echoed_credentials_nested_arrays_and_urls_are_masked_and_scopes_reset() {
        let (_, replies) = capture(|| {
            record(
                "GET",
                "https://user:pass@example.com/a?token=short",
                Some(200),
                "application/json",
                r#"{"nested":{"access_token":["hidden","also-hidden"]},"echo":"small","hex":"abcdef1234567890abcdef1234567890","mail":"someone@example.org"}"#,
                &[("Cookie", "session=small")],
                None,
            )
        });
        assert_eq!(replies.len(), 1);
        for secret in [
            "hidden",
            "small",
            "abcdef1234",
            "someone@",
            "user:pass",
            "token=short",
        ] {
            assert!(!format!("{:?}", replies).contains(secret));
        }
        assert!(capture(|| ()).1.is_empty());
        let _ = std::panic::catch_unwind(|| capture(|| panic!("cancel")));
        assert!(capture(|| ()).1.is_empty());
    }
    #[test]
    fn malformed_and_truncated_replies_mask_whole_quoted_credentials() {
        let (_, replies) = capture(|| {
            record(
                "GET",
                "https://example.com/a%2Bb%2Fc",
                Some(200),
                "text/plain",
                r#"password="multiple secret words"; token="truncated secret"#,
                &[("Authorization", "a+b/c")],
                None,
            );
        });
        for secret in ["multiple", "secret words", "truncated", "a%2Bb%2Fc"] {
            assert!(!format!("{:?}", replies).contains(secret));
        }
    }
}
