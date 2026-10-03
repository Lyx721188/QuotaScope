//! Local navigation links carry actions only, never credentials or commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Settings,
    Dashboard,
    Refresh,
}
pub fn link(raw: &str) -> Option<Action> {
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "quotascope"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if url.host_str()? == "account" {
        let id = url.path().strip_prefix('/')?.replace("%23", "#");
        if id.len() > 256
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"#-_.".contains(&b))
        {
            return None;
        }
        return crate::model::AccountKey::from_id(&id).map(|_| Action::Settings);
    }
    if !matches!(url.path(), "" | "/") {
        return None;
    }
    match url.host_str()? {
        "settings" => Some(Action::Settings),
        "dashboard" => Some(Action::Dashboard),
        "refresh" => Some(Action::Refresh),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_links_reject_credentials_parameters_and_commands() {
        assert_eq!(link("quotascope://dashboard"), Some(Action::Dashboard));
        assert_eq!(
            link("quotascope://account/codex%231"),
            Some(Action::Settings)
        );
        for raw in [
            "https://dashboard",
            "quotascope://refresh?token=secret",
            "quotascope://user@settings",
            "quotascope://settings/run",
            "quotascope://exec",
        ] {
            assert_eq!(link(raw), None);
        }
    }
}
