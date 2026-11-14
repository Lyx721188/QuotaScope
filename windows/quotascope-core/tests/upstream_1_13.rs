//! Synthetic protocol regressions against Pulse 338c796; no live accounts.
use chrono::NaiveDate;
use quotascope_core::model::{AccountKey, CreditAmount, Kind, Provider, State};
use quotascope_core::providers::{cursor, session_spec, Services};
use quotascope_core::service_status::{self, Page};
use quotascope_core::spend::Span;
use serde_json::json;

#[test]
fn catalog_and_credential_routes_are_connected() {
    let services = Services::new();
    for p in quotascope_core::model::ALL_PROVIDERS {
        assert!(p.is_ported_to_windows(), "{p:?}");
        assert!(services.for_provider(p).is_some(), "{p:?}");
        assert_eq!(Provider::from_raw(p.raw()), Some(p));
        if session_spec(p).is_some() {
            assert!(p.uses_api_key(), "{p:?} must reload its encrypted session");
        }
    }
    assert_eq!(Provider::from_raw("openDesign"), Some(Provider::OpenDesign));
    assert_eq!(Provider::from_raw("dimAgent"), Some(Provider::DimAgent));
    assert!(quotascope_core::accounts::supports(Provider::OpenDesign));
    assert!(!quotascope_core::accounts::supports(Provider::DimAgent));
}

#[test]
fn weekly_cursor_plans_replace_monthly_pools_but_keep_on_demand() {
    let summary = json!({"individualUsage":{"plan":{"autoPercentUsed":0,"apiPercentUsed":0},"onDemand":{"enabled":true,"used":20,"limit":100}}});
    let plan = json!({"planInfo":{"includedUsagePeriod":"INCLUDED_USAGE_PERIOD_WEEKLY","includedUsageResetsAt":"1794528000000"}});
    assert!(cursor::weekly_plan(&plan));
    assert!(cursor::weekly_plan(
        &json!({"planInfo":{"includedUsagePeriod":2}})
    ));
    assert!(!cursor::weekly_plan(
        &json!({"planInfo":{"includedUsagePeriod":1}})
    ));
    let windows = cursor::weekly_windows(
        &plan,
        &json!({"planUsage":{"totalPercentUsed":25}}),
        cursor::windows_from_reply(&summary),
    );
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].kind, Kind::Weekly);
    assert_eq!(windows[0].used_fraction, 0.25);
    assert_eq!(windows[0].resets_at, Some(1794528000000));
    assert_eq!(windows[1].kind, Kind::Spend);
    let missing = cursor::weekly_windows(&plan, &json!({}), cursor::windows_from_reply(&summary));
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].kind, Kind::Spend);
}

#[test]
fn cursor_enterprise_overall_is_a_single_monthly_allowance() {
    let windows = cursor::windows_from_reply(
        &json!({"individualUsage":{"overall":{"used":2500,"limit":10000,"remaining":7500}}}),
    );
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].kind, Kind::Monthly);
    assert_eq!(windows[0].used_fraction, 0.25);
    assert!(!windows[0].reports_length);
}

#[test]
fn prepaid_xiaomi_retains_currency_and_does_not_invent_a_plan() {
    let usage = quotascope_core::providers::xiaomi_mimo::balance_reading(
        CreditAmount {
            amount: 12.34,
            currency: "CNY".into(),
        },
        AccountKey::primary(Provider::XiaomiMiMo),
    );
    assert_eq!(usage.state, State::Live);
    assert!(usage.windows.is_empty());
    assert!(usage.plan.is_none());
    assert_eq!(usage.credit_remaining.unwrap().currency, "CNY");
}

#[test]
fn calendar_ranges_handle_year_boundaries_and_custom_inclusive_dates() {
    let date = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
    let today = date("2026-01-02");
    assert!(Span::LastMonth.contains(date("2025-12-01"), today));
    assert!(Span::LastMonth.contains(date("2025-12-31"), today));
    assert!(!Span::LastMonth.contains(date("2026-01-01"), today));
    assert!(Span::ThisMonth.contains(date("2026-01-01"), today));
    assert!(!Span::ThisYear.contains(date("2025-12-31"), today));
    let span = Span::Custom {
        start: date("2025-12-31"),
        end: today,
    };
    assert!(span.contains(date("2025-12-31"), today));
    assert!(span.contains(today, today));
    assert!(!span.contains(date("2026-01-03"), today));
    assert_eq!(span.selection_index(), 8);
    assert_eq!(
        serde_json::from_str::<Span>(&serde_json::to_string(&span).unwrap()).unwrap(),
        span
    );
    assert_eq!(
        serde_json::from_str::<Span>("\"week\"").unwrap(),
        Span::Week
    );
}

#[test]
fn statuspage_rules_separate_cursor_from_grok_and_ignore_suppliers() {
    let bytes = serde_json::to_vec(&json!({"components":[
        {"id":"rflc60xp5jp2","name":"IDE","status":"operational"},
        {"id":"sm5wkcnqkvr9","name":"Grok Bot","status":"major_outage"},
        {"id":"website","name":"Website","status":"major_outage"}
    ]}))
    .unwrap();
    let cursor = service_status::parse(Page::Cursor, &bytes, 1).unwrap();
    assert_eq!(cursor.components.len(), 2);
    assert!(Page::Cursor.notifies_about(&cursor.components[0]));
    assert!(!Page::Cursor.notifies_about(&cursor.components[1]));
    let grok = service_status::parse(Page::GrokBot, &bytes, 1).unwrap();
    assert_eq!(grok.components.len(), 1);
    assert!(Page::GrokBot.notifies_about(&grok.components[0]));
    let bytes = serde_json::to_vec(&json!({"components":[
        {"id":"api","name":"Factory API","status":"operational"},
        {"id":"supplier","group_id":"c9sjq8nsz2kq","name":"External model","status":"major_outage"}
    ]}))
    .unwrap();
    assert_eq!(
        service_status::parse(Page::Factory, &bytes, 1)
            .unwrap()
            .components
            .len(),
        1
    );
    assert_eq!(Page::ALL.len(), 45);
    assert_eq!(Page::for_provider(Provider::Windsurf), Some(Page::Windsurf));
}

#[test]
fn only_documented_flash_arms_share_the_published_price() {
    for model in ["gemini-3.8-flash-n", "gemini-3.8-flash-xpe"] {
        assert!(
            quotascope_core::model_prices::aliases(model).contains(&"gemini-3.8-flash".to_string())
        );
    }
    assert!(
        !quotascope_core::model_prices::aliases("gemini-3.8-flash-exp-b")
            .contains(&"gemini-3.8-flash".to_string())
    );
}
