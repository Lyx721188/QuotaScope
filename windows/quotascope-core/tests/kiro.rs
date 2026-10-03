//! Pulse fixture at 3696a65, not a response captured from a Windows account.
use quotascope_core::model::{AccountKey, Kind, Provider, State, Unavailability};
use quotascope_core::providers::{kiro, session_spec, Services};
use serde_json::{json, Value};

const NOW: i64 = 1_791_024_000_000;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/upstream-kiro-pro-plus-usage.json")).unwrap()
}

fn reply(rows: Value) -> Value {
    json!({"success":true,"data":{"usageBreakdowns":rows}})
}

#[test]
fn upstream_fixture_preserves_exact_credit_counts_plan_and_reset() {
    let usage = kiro::reading_at(&fixture(), NOW);
    assert_eq!(usage.state, State::Live);
    assert_eq!(usage.account, AccountKey::primary(Provider::Kiro));
    assert_eq!(usage.origin.as_deref(), Some("kiroACP"));
    assert_eq!(usage.plan.as_deref(), Some("KIRO PRO+"));
    assert_eq!(usage.observed_at, Some(NOW));
    assert_eq!(usage.windows.len(), 2);
    assert_eq!(usage.windows[0].id, "credit");
    assert_eq!(usage.windows[1].id, "bonus_credit");
    assert_eq!(usage.windows[0].scope.as_deref(), Some("Credits"));
    assert!((usage.windows[0].used_fraction - 0.061725).abs() < 1e-12);
    assert_eq!(usage.windows[1].used_fraction, 0.25);
    let reset = chrono::NaiveDate::from_ymd_opt(2099, 10, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    for window in &usage.windows {
        assert_eq!(window.kind, Kind::Monthly);
        assert_eq!(window.resets_at, Some(reset));
        assert!(!window.reports_length);
        assert_eq!(window.elapsed_fraction(NOW), None);
        assert_eq!(window.estimate, None);
        assert!(!window.is_exhausted);
    }
    assert!(usage.credit_balance.is_none());
    assert!(usage.credit_remaining.is_none());
}

#[test]
fn percentages_need_a_provider_limit_and_never_replace_reported_used() {
    let usage = kiro::reading_at(
        &reply(json!([
            {"resourceType":"USED","used":1,"limit":10,"percentage":90},
            {"resourceType":"PERCENT","percentage":12.5,"limit":80},
            {"resourceType":"NO_LIMIT","percentage":50},
            {"resourceType":"ZERO","used":1,"limit":0},
            {"resourceType":"NEGATIVE_LIMIT","used":1,"limit":-1},
            {"resourceType":"UNBOUNDED","used":1,"limit":10,"hasLimit":false},
            {"resourceType":"UNKNOWN_USED","limit":10}
        ])),
        NOW,
    );
    assert_eq!(usage.windows.len(), 2);
    assert_eq!(usage.windows[0].used_fraction, 0.1);
    assert_eq!(usage.windows[1].used_fraction, 0.125);
    assert!(usage.windows.iter().all(|w| w.resets_at.is_none()));
    assert_eq!(
        kiro::reading_at(&reply(json!([])), NOW).state,
        State::Unavailable(Unavailability::NoLimitsReported)
    );
}

#[test]
fn pool_ids_survive_reordering_and_duplicate_resources_are_disambiguated() {
    let original = kiro::reading_at(&fixture(), NOW);
    let mut reordered = fixture();
    reordered["data"]["usageBreakdowns"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let reordered = kiro::reading_at(&reordered, NOW);
    for window in original.windows {
        assert_eq!(
            reordered.windows.iter().find(|w| w.id == window.id),
            Some(&window)
        );
    }
    let usage = kiro::reading_at(
        &reply(json!([
            {"resourceType":" CREDIT ","used":1,"limit":2},
            {"resourceType":"credit","used":2,"limit":3},
            {"resourceType":" ","displayName":" Specs ","used":1,"limit":4},
            {"used":1,"limit":5}, {"used":1,"limit":6}
        ])),
        NOW,
    );
    assert_eq!(
        usage
            .windows
            .iter()
            .map(|w| w.id.as_str())
            .collect::<Vec<_>>(),
        ["credit", "credit.2", "specs", "usage", "usage.2"]
    );
    assert_eq!(usage.windows[0].scope.as_deref(), Some(" CREDIT "));
}

#[test]
fn exhaustion_uses_unclamped_counts_and_malformed_dates_stay_unknown() {
    let mut root = reply(json!([
        {"used":-1,"limit":10}, {"used":10,"limit":10}, {"used":11,"limit":10}
    ]));
    for text in [
        "not-a-date",
        "2026-02-30",
        "2026-1-01",
        "2026-10-01T00:00:00Z",
    ] {
        root["data"]["billingCycleReset"] = json!(text);
        let usage = kiro::reading_at(&root, NOW);
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|w| w.used_fraction)
                .collect::<Vec<_>>(),
            [0.0, 1.0, 1.0]
        );
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|w| w.is_exhausted)
                .collect::<Vec<_>>(),
            [false, true, true]
        );
        assert!(usage.windows.iter().all(|w| w.resets_at.is_none()));
    }
}

#[test]
fn malformed_envelopes_and_types_are_not_silently_partially_decoded() {
    for root in [
        json!({}),
        json!({"success":"true"}),
        json!({"success":true,"data":{}}),
        reply(json!([{"used":"1","limit":2}])),
        reply(json!([{"used":1,"limit":"2"}])),
        reply(json!([{"used":1,"limit":2,"hasLimit":"true"}])),
        reply(json!([{"used":1,"limit":2,"percentage":"50"}])),
        reply(json!([{"used":1,"limit":2,"resourceType":3}])),
        json!({"success":true,"data":{"usageBreakdowns":null}}),
    ] {
        assert_eq!(
            kiro::reading_at(&root, NOW).state,
            State::Unavailable(Unavailability::UnreadableReply),
            "{root}"
        );
    }
    assert_eq!(
        kiro::reading_at(&json!({"success":true}), NOW).state,
        State::Unavailable(Unavailability::NoLimitsReported)
    );
    let mut valid = fixture();
    valid["data"]["usageBreakdowns"][1]["used"] = json!("25");
    assert_eq!(
        kiro::reading_at(&valid, NOW).state,
        State::Unavailable(Unavailability::UnreadableReply)
    );
}

#[test]
fn unsuccessful_replies_surface_login_version_and_unknown_failures() {
    for (message, reason) in [
        ("Please sign in", Unavailability::KiroSignInRequired),
        ("Not authenticated", Unavailability::KiroSignInRequired),
        ("login required", Unavailability::KiroSignInRequired),
        ("Method not found", Unavailability::KiroVersionUnsupported),
        (
            "Unsupported agent-engine",
            Unavailability::KiroVersionUnsupported,
        ),
        ("Something changed", Unavailability::UnreadableReply),
    ] {
        let usage = kiro::reading_at(&json!({"success":false,"message":message}), NOW);
        assert_eq!(usage.state, State::Unavailable(reason));
        assert_eq!(usage.origin.as_deref(), Some("kiroACP"));
    }
}

#[test]
fn catalog_registry_and_legacy_settings_enable_kiro_without_a_saved_key() {
    let services = Services::new();
    assert_eq!(
        services
            .list
            .iter()
            .filter(|s| s.provider() == Provider::Kiro)
            .count(),
        1
    );
    assert_eq!(
        services
            .for_provider(Provider::Kiro)
            .unwrap()
            .origin_token(),
        "kiroACP"
    );
    assert!(Provider::Kiro.is_ported_to_windows());
    assert_eq!(Provider::Kiro.windows_gap(), None);
    assert!(!Provider::Kiro.keeps_own_credential());
    assert_eq!(session_spec(Provider::Kiro), None);
    let settings: quotascope_core::settings::AppSettings = serde_json::from_value(json!({
        "enabledAccounts":["kiro"],"providerOrder":["kiro"]
    }))
    .unwrap();
    assert_eq!(
        settings.ordered_enabled(),
        vec![AccountKey::primary(Provider::Kiro)]
    );
}
