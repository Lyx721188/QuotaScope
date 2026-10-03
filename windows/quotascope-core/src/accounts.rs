//! Additional accounts keep their own encrypted credential. They never borrow
//! the primary account's login or local usage history.
use crate::model::{AccountKey, Provider};
pub fn supports(provider: Provider) -> bool {
    provider.uses_api_key() || provider == Provider::ClaudeCode
}
pub fn add(provider: Provider, credential: &str) -> Option<AccountKey> {
    if !supports(provider) || credential.trim().is_empty() {
        return None;
    }
    if provider == Provider::ClaudeCode {
        let value: serde_json::Value = serde_json::from_str(credential).ok()?;
        value["claudeAiOauth"]["accessToken"]
            .as_str()
            .filter(|s| !s.is_empty())?;
    }
    let mut slot = crate::settings::with(|s| s.next_account_slot.max(1));
    let account = loop {
        let account = AccountKey {
            provider,
            slot: slot.to_string(),
        };
        if !crate::settings::with(|s| s.enabled_accounts.contains(&account.id())) {
            break account;
        }
        slot = slot.checked_add(1)?;
    };
    crate::secrets::set_key(&account.id(), credential.trim());
    // Do not create a ring if encrypted persistence failed.
    if crate::secrets::key_for(&account.id()).as_deref() != Some(credential.trim()) {
        return None;
    }
    crate::settings::mutate(|s| {
        s.enabled_accounts.insert(account.id());
        s.next_account_slot = slot.saturating_add(1);
    });
    Some(account)
}
pub fn remove(account: &AccountKey) {
    if account.is_primary() {
        return;
    }
    crate::secrets::set_key(&account.id(), "");
    crate::settings::mutate(|s| {
        let id = account.id();
        s.enabled_accounts.remove(&id);
        s.pinned_windows.remove(&id);
        s.detailed_cards.remove(&id);
        s.ring_tints.remove(&id);
        s.balance_bases.remove(&id);
        s.balance_budgets.remove(&id);
        s.low_balance_alerts.remove(&id);
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn additional_accounts_keep_only_their_credential_and_never_borrow_sessions() {
        let account = AccountKey::from_id("claudeCode#1").unwrap();
        let mut keys = crate::providers::KeyRing::default();
        keys.api_keys.insert("claudeCode#1".into(), "own".into());
        keys.api_keys.insert("claudeCode".into(), "primary".into());
        keys.api_keys.insert(
            crate::providers::claude_session::SECRET.into(),
            "web".into(),
        );
        keys.opencode_console = Some("console".into());
        keys.copilot_token = Some("copilot".into());
        let isolated = keys.for_account(&account);
        assert_eq!(isolated.api_keys.len(), 1);
        assert_eq!(
            isolated.api_key(Provider::ClaudeCode).as_deref(),
            Some("own")
        );
        assert!(isolated.opencode_console.is_none() && isolated.copilot_token.is_none());
        assert!(keys
            .for_account(&AccountKey::from_id("claudeCode#2").unwrap())
            .api_keys
            .is_empty());
    }
}
