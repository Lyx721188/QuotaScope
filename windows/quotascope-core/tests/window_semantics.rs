use quotascope_core::model::{Kind, UsageWindow};

fn reading(kind: Kind, used: f64, reset: Option<i64>) -> UsageWindow {
    UsageWindow::new("allowance", kind, None, used, 86_400, reset)
}

#[test]
fn buying_credit_or_a_top_up_never_claims_a_reset() {
    for kind in [
        Kind::Balance,
        Kind::TopUp,
        Kind::Credits,
        Kind::SharedCredits,
    ] {
        let previous = reading(kind, 0.95, Some(100_000));
        let current = reading(kind, 0.1, Some(100_000));
        assert!(!current.has_turned_over(&previous));
        let refilled = reading(kind, 0.1, Some(200_000));
        assert_eq!(
            refilled.has_turned_over(&previous),
            matches!(kind, Kind::Credits | Kind::SharedCredits)
        );
    }
}

#[test]
fn rolling_limits_do_not_reset_from_small_falls_or_timestamp_jitter() {
    let previous = reading(Kind::Weekly, 0.9, Some(100_000));
    assert!(!reading(Kind::Weekly, 0.89, Some(160_000)).has_turned_over(&previous));
    assert!(reading(Kind::Weekly, 0.89, Some(160_001)).has_turned_over(&previous));
    assert!(reading(Kind::Weekly, 0.4, None).has_turned_over(&previous));
    assert!(!reading(Kind::Weekly, 0.8, None).has_turned_over(&previous));
}

#[test]
fn legacy_window_caches_remain_readable_without_expiring_amount() {
    let mut value = serde_json::to_value(reading(Kind::Monthly, 0.5, None)).unwrap();
    value.as_object_mut().unwrap().remove("next_expiry_amount");
    let window: UsageWindow = serde_json::from_value(value).unwrap();
    assert_eq!(window.next_expiry_amount, None);
    for (kind, token) in [
        (Kind::Daily, "daily"),
        (Kind::Messages, "messages"),
        (Kind::TopUp, "topUp"),
        (Kind::Credits, "credits"),
        (Kind::SharedCredits, "sharedCredits"),
    ] {
        assert_eq!(kind.token(), token);
    }
}

#[test]
fn packs_expiring_on_one_calendar_day_are_aggregated_without_expired_or_empty_parts() {
    use chrono::{Local, TimeZone};
    use quotascope_core::model::AllowanceExpiry;
    let at = Local
        .with_ymd_and_hms(2026, 10, 2, 10, 0, 0)
        .unwrap()
        .timestamp_millis();
    let parts = [
        (100.0, at),
        (200.0, at + 3_600_000),
        (500.0, at + 86_400_000),
        (1000.0, at - 86_400_000),
        (0.0, at),
        (f64::NAN, at),
    ];
    let expiry = AllowanceExpiry::soonest(parts.into_iter(), at - 1000).unwrap();
    assert_eq!(expiry.at, at);
    assert_eq!(expiry.amount, 300.0);
    assert!(AllowanceExpiry::soonest([(1.0, at)].into_iter(), at).is_none());
}
