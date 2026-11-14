//! A storefront is part of a credential's identity, never a fallback host.
use crate::model::{AccountKey, Provider};
use crate::settings::AppSettings;
pub fn default_site(provider: Provider) -> &'static str {
    if provider == Provider::StepFun || provider == Provider::MinimaxCn {
        "china"
    } else {
        "international"
    }
}
pub fn site(account: &AccountKey, settings: &AppSettings) -> String {
    if !account.is_primary() {
        return settings
            .account_sites
            .get(&account.id())
            .cloned()
            .unwrap_or_else(|| default_site(account.provider).into());
    }
    match account.provider {
        Provider::Minimax => settings.minimax_site.clone(),
        Provider::MinimaxCn => "china".into(),
        Provider::Qoder => settings.qoder_site.clone(),
        Provider::StepFun => settings.stepfun_site.clone(),
        _ => default_site(account.provider).into(),
    }
}
pub fn slot_for(account: &AccountKey, settings: &AppSettings) -> String {
    if !account.is_primary() {
        return account.id();
    }
    let site = site(account, settings);
    match account.provider {
        Provider::Minimax if site == "china" => "minimaxCN".into(),
        Provider::Qoder if site == "china" => "qoderCN".into(),
        Provider::StepFun if site == "international" => "stepFunInternational".into(),
        _ => account.id(),
    }
}
pub fn slot(raw: &str) -> String {
    AccountKey::from_id(raw)
        .map(|a| crate::settings::with(|s| slot_for(&a, s)))
        .unwrap_or_else(|| raw.into())
}
pub fn context(account: &AccountKey, site: &str, key: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{}:{site}:{:x}",
        account.id(),
        Sha256::digest(key.unwrap_or("").as_bytes())
    )
}
pub fn current_context(account: &AccountKey) -> String {
    let (site, slot) = crate::settings::with(|s| (site(account, s), slot_for(account, s)));
    let mut key = crate::secrets::key_for(&slot).unwrap_or_default();
    if account.provider == Provider::DeepSeek {
        key.push('|');
        key.push_str(&crate::deepseek_session::token(account).unwrap_or_default());
    }
    if account.provider == Provider::OpenCodeGo && account.is_primary() {
        key.push('|');
        key.push_str(&crate::secrets::key_for(crate::opencode_console::SECRET).unwrap_or_default());
    }
    context(account, &site, Some(&key))
}

/// Keys remain in their original DPAPI slots: minimax and minimaxCN. Added
/// accounts move only after the new encrypted entry has been read back.
pub fn migrate_minimax() {
    let mut settings = crate::settings::with(Clone::clone);
    if settings.merged_minimax_cn {
        return;
    }
    let retired: Vec<_> = settings
        .enabled_accounts
        .iter()
        .filter(|id| id.starts_with("minimaxCN#"))
        .cloned()
        .collect();
    for old in retired {
        let mut new = old.replacen("minimaxCN#", "minimax#", 1);
        while settings.enabled_accounts.contains(&new) {
            let slot = settings.next_account_slot.max(1);
            settings.next_account_slot = match slot.checked_add(1) {
                Some(next) => next,
                None => return,
            };
            new = format!("minimax#{slot}");
        }
        if let Some(key) = crate::secrets::key_for(&old) {
            crate::secrets::set_key(&new, &key);
            if crate::secrets::key_for(&new).as_deref() != Some(key.as_str()) {
                return;
            }
            // Keep the retired encrypted slot as recovery data. It is no longer
            // read by the migrated account; a failed settings save loses no key.
        }
        fold_account(&mut settings, &old, &new, true);
        settings.account_sites.insert(new, "china".into());
    }
    fold_minimax(&mut settings);
    crate::settings::mutate(|current| *current = settings);
}
fn fold_map<T: Clone>(
    map: &mut std::collections::HashMap<String, T>,
    old: &str,
    new: &str,
    wins: bool,
) {
    if let Some(value) = map.remove(old) {
        if wins || !map.contains_key(new) {
            map.insert(new.into(), value);
        }
    }
}
fn fold_set(set: &mut std::collections::HashSet<String>, old: &str, new: &str, wins: bool) {
    if set.remove(old) && wins {
        set.insert(new.into());
    }
}
fn fold_list(list: &mut Vec<String>, old: &str, new: &str, wins: bool) {
    if !list.iter().any(|id| id == old) {
        return;
    }
    if wins {
        list.retain(|id| id != new);
        for id in list.iter_mut() {
            if id == old {
                *id = new.into();
            }
        }
    } else {
        list.retain(|id| id != old);
    }
    let mut seen = std::collections::HashSet::new();
    list.retain(|id| seen.insert(id.clone()));
}
fn fold_account(s: &mut AppSettings, old: &str, new: &str, wins: bool) {
    let enabled = s.enabled_accounts.remove(old);
    if enabled {
        s.enabled_accounts.insert(new.into());
    }
    fold_list(&mut s.provider_order, old, new, wins);
    fold_list(&mut s.offered_providers, old, new, wins);
    fold_set(&mut s.detailed_cards, old, new, wins);
    fold_map(&mut s.ring_tints, old, new, wins);
    fold_map(&mut s.robot_avatars, old, new, wins);
    fold_map(&mut s.shown_model_groups, old, new, wins);
    fold_set(&mut s.split_accounts, old, new, wins);
    fold_set(&mut s.credit_amounts, old, new, wins);
    fold_map(&mut s.low_balance_alerts, old, new, wins);
    fold_map(&mut s.balance_bases, old, new, wins);
    fold_map(&mut s.balance_budgets, old, new, wins);
    fold_map(&mut s.server_addresses, old, new, wins);
    fold_map(&mut s.account_sites, old, new, wins);
    if let Some(mut pin) = s.pinned_windows.remove(old) {
        if wins || !s.pinned_windows.contains_key(new) {
            if pin.starts_with("minimaxCN.") {
                pin = pin.replacen("minimaxCN.", "minimax.", 1);
            }
            s.pinned_windows.insert(new.into(), pin);
        }
    }
}
pub fn fold_minimax(s: &mut AppSettings) {
    let wins = s.enabled_accounts.contains("minimaxCN") && !s.enabled_accounts.contains("minimax");
    if wins {
        s.minimax_site = "china".into();
    }
    fold_account(s, "minimaxCN", "minimax", wins);
    s.merged_minimax_cn = true;
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retired_only_minimax_account_keeps_its_site_position_and_pinned_lane() {
        let mut s = AppSettings::default();
        s.enabled_accounts.insert("minimaxCN".into());
        s.provider_order = vec!["codex".into(), "minimaxCN".into(), "minimax".into()];
        s.ring_tints.insert("minimaxCN".into(), "#123456".into());
        s.pinned_windows
            .insert("minimaxCN".into(), "minimaxCN.general.weekly".into());
        s.detailed_cards.insert("minimaxCN".into());
        fold_minimax(&mut s);
        assert_eq!(s.minimax_site, "china");
        assert_eq!(s.provider_order, vec!["codex", "minimax"]);
        assert!(s.enabled_accounts.contains("minimax"));
        assert!(!s.enabled_accounts.contains("minimaxCN"));
        assert_eq!(s.pinned_windows["minimax"], "minimax.general.weekly");
        assert!(s.detailed_cards.contains("minimax"));
        assert!(s.merged_minimax_cn);
    }
    #[test]
    fn an_existing_international_account_keeps_its_own_preferences() {
        let mut s = AppSettings::default();
        s.enabled_accounts
            .extend(["minimax".into(), "minimaxCN".into()]);
        s.ring_tints.insert("minimax".into(), "#112233".into());
        s.ring_tints.insert("minimaxCN".into(), "#445566".into());
        fold_minimax(&mut s);
        assert_eq!(s.minimax_site, "international");
        assert_eq!(s.ring_tints["minimax"], "#112233");
    }
    #[test]
    fn primary_sites_keep_separate_slots_and_added_accounts_keep_their_site() {
        let mut settings = AppSettings::default();
        let account = AccountKey::primary(Provider::Qoder);
        assert_eq!(slot_for(&account, &settings), "qoder");
        settings.qoder_site = "china".into();
        assert_eq!(slot_for(&account, &settings), "qoderCN");
        let added = AccountKey::from_id("qoder#1").unwrap();
        settings
            .account_sites
            .insert(added.id(), "international".into());
        assert_eq!(site(&added, &settings), "international");
        assert_eq!(slot_for(&added, &settings), "qoder#1");
    }
    #[test]
    fn a_site_or_credential_change_invalidates_its_context() {
        let a = AccountKey::primary(Provider::Minimax);
        assert_ne!(
            context(&a, "china", Some("one")),
            context(&a, "international", Some("one"))
        );
        assert_ne!(
            context(&a, "china", Some("one")),
            context(&a, "china", Some("two"))
        );
        assert!(!context(&a, "china", Some("credential-secret")).contains("credential-secret"));
    }
}
