use quotascope_core::model::{AccountKey, Provider, ALL_PROVIDERS};
use quotascope_core::settings::AppSettings;

#[test]
fn usage_pages_are_the_eighteen_known_https_pages() {
    let pages: Vec<_> = ALL_PROVIDERS
        .iter()
        .filter_map(|p| p.usage_page())
        .collect();
    assert_eq!(pages.len(), 18);
    assert!(pages
        .iter()
        .all(|page| url::Url::parse(page).unwrap().scheme() == "https"));
    assert_eq!(
        Provider::OllamaCloud.usage_page(),
        Some("https://ollama.com/settings")
    );
    assert_eq!(Provider::Extension.usage_page(), None);
    assert_eq!(Provider::Zai.usage_page(), None);
}

#[test]
fn billing_groups_do_not_confuse_api_keys_with_api_accounts() {
    assert!(Provider::Zai.uses_api_key());
    assert!(!Provider::Zai.is_api_billing());
    assert!(!Provider::Amp.is_api_billing());
    assert!(Provider::Replicate.is_api_billing());
    assert!(Provider::Replicate.reports_spendable_balance());
    assert!(Provider::DeepSeek.is_api_billing());
    assert!(Provider::Bifrost.is_api_billing());
}

#[test]
fn old_settings_preserve_keys_and_default_to_compact_cards() {
    let settings: AppSettings = serde_json::from_str(
        r#"{"enabledAccounts":["codex"],"deepseekBasis":"budget","deepseekBudget":200}"#,
    )
    .unwrap();
    assert!(settings.detailed_cards.is_empty());
    assert!(!settings.reads_token_spend);
    assert_eq!(settings.deepseek_budget, Some(200.0));
    assert_eq!(
        settings.ordered_enabled(),
        vec![AccountKey::primary(Provider::Codex)]
    );
    let mut detailed = settings;
    detailed.detailed_cards.insert("codex".into());
    let restored: AppSettings =
        serde_json::from_str(&serde_json::to_string(&detailed).unwrap()).unwrap();
    assert!(restored.detailed_cards.contains("codex"));
    assert!(!restored.detailed_cards.contains("claudeCode"));
}
