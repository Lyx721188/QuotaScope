use quotascope_core::model::{burn_rate, Kind, UsageWindow};

const NOW: i64 = 1_700_000_000_000;

fn window(seconds: i64, elapsed: f64, used: f64) -> UsageWindow {
    UsageWindow::new(
        "forecast",
        Kind::FiveHour,
        None,
        used,
        seconds,
        Some(NOW + (seconds as f64 * 1000.0 * (1.0 - elapsed)).round() as i64),
    )
}

#[test]
fn screenshot_with_39_percent_used_at_17_percent_elapsed_runs_out_in_80_minutes() {
    let forecast = burn_rate::reading(&window(5 * 3600, 0.17, 0.39), NOW).unwrap();
    assert!(forecast.exhausts_before_reset);
    let remaining = forecast.time_to_exhaustion_ms.unwrap();
    // 51 minutes elapsed, with 61/39 as much quota left as has been used.
    let expected = (51.0 * 60_000.0 * 61.0 / 39.0) as i64;
    assert!((remaining - expected).abs() <= 1);
}

#[test]
fn weekly_forecast_distinguishes_reset_boundary_and_exhaustion_beyond_eta_horizon() {
    let seconds = 7 * 86400;
    for used in [0.0, 0.3, 0.6] {
        let forecast = burn_rate::reading(&window(seconds, 0.6, used), NOW).unwrap();
        assert!(!forecast.exhausts_before_reset);
        assert_eq!(forecast.time_to_exhaustion_ms, None);
    }
    let forecast = burn_rate::reading(&window(seconds, 0.6, 0.7), NOW).unwrap();
    assert!(forecast.exhausts_before_reset);
    assert_eq!(forecast.time_to_exhaustion_ms, None); // More than two hours away.
}

#[test]
fn prediction_requires_reported_length_valid_reset_and_enough_elapsed_time() {
    assert!(burn_rate::reading(&window(18000, 0.02, 0.4), NOW).is_none());
    assert!(burn_rate::reading(&window(18000, 1.0, 0.4), NOW).is_none());
    let mut input = window(18000, 0.17, 0.39);
    input.reports_length = false;
    assert!(burn_rate::reading(&input, NOW).is_none());
    input.reports_length = true;
    input.resets_at = None;
    assert!(burn_rate::reading(&input, NOW).is_none());
}

#[test]
fn spent_quota_has_zero_time_left_and_zero_usage_has_no_exhaustion() {
    let spent = burn_rate::reading(&window(18000, 0.17, 1.0), NOW).unwrap();
    assert!(spent.exhausts_before_reset);
    assert_eq!(spent.time_to_exhaustion_ms, Some(0));
    let unused = burn_rate::reading(&window(18000, 0.17, 0.0), NOW).unwrap();
    assert!(!unused.exhausts_before_reset);
    assert_eq!(unused.time_to_exhaustion_ms, None);
}
