//! Explicitly imported Claude Desktop/web session, separate from CLI OAuth.
use crate::http::{HttpClient, Method};
use crate::model::{AccountKey, Provider, ProviderUsage, Unavailability};
pub const SECRET: &str = "claudeCode:webSession";
pub const COOKIES: &[&str] = &["sessionKey", "lastActiveOrg"];
fn identity(header: &str) -> Option<(String, Option<String>)> {
    let mut session = None;
    let mut active = None;
    if header.len() > 32768 || header.chars().any(char::is_control) {
        return None;
    }
    for part in header.split(';') {
        let (name, value) = part.trim().split_once('=')?;
        match name {
            "sessionKey" if !value.is_empty() => session = Some(value),
            "lastActiveOrg" if !value.is_empty() => active = Some(value.to_string()),
            _ => {}
        }
    }
    Some((format!("sessionKey={}", session?), active))
}
fn organization(root: &serde_json::Value, active: Option<&str>) -> Option<String> {
    let rows = root["account"]["memberships"].as_array()?;
    let organizations: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["organization"]["uuid"].as_str())
        .collect();
    let id = if let Some(active) = active {
        organizations.iter().copied().find(|id| *id == active)?
    } else if organizations.len() == 1 {
        organizations[0]
    } else {
        return None;
    };
    (id.len() <= 128
        && !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    .then(|| id.to_string())
}
pub fn fetch(http: &HttpClient, header: &str) -> Result<ProviderUsage, Unavailability> {
    let (cookie, active) = identity(header).ok_or(Unavailability::SessionExpired)?;
    let bootstrap = http.fetch_json(
        Method::Get,
        "https://claude.ai/api/bootstrap",
        &[("Cookie", &cookie)],
        None,
    )?;
    let org =
        organization(&bootstrap, active.as_deref()).ok_or(Unavailability::NoLimitsReported)?;
    let reply = http.fetch_json(
        Method::Get,
        &format!("https://claude.ai/api/organizations/{org}/usage"),
        &[("Cookie", &cookie)],
        None,
    )?;
    let windows = super::claude_code::parse_windows(&reply);
    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(AccountKey::primary(Provider::ClaudeCode), windows);
    usage.origin = Some("webSession".into());
    Ok(usage)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_organization_is_required_for_ambiguous_memberships() {
        let root = serde_json::json!({"account":{"memberships":[{"organization":{"uuid":"personal"}},{"organization":{"uuid":"work"}}]}});
        assert_eq!(organization(&root, None), None);
        assert_eq!(organization(&root, Some("work")), Some("work".into()));
        assert_eq!(organization(&root, Some("gone")), None);
        assert_eq!(
            identity("analytics=private; sessionKey=auth; lastActiveOrg=work"),
            Some(("sessionKey=auth".into(), Some("work".into())))
        );
        assert!(identity("sessionKey=auth\r\nX-Other: injected").is_none());
    }
}
