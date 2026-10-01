use quotascope_core::model::State;
use quotascope_core::providers::codex::parse_app_server_response;
use serde_json::json;

#[test]
fn exhaustion_is_scoped_to_its_group_and_a_missing_allowance_flag_does_not_infer_it() {
    let usage = parse_app_server_response(&json!({"rateLimitsByLimitId":{
        "account":{"primary":{"usedPercent":99,"windowDurationMins":300}},
        "model":{"limitName":"Model","primary":{"usedPercent":70,"windowDurationMins":300},
            "secondary":{"usedPercent":20,"windowDurationMins":10080},"spendControlReached":true}
    }}));
    assert_eq!(usage.windows.len(), 3);
    assert!(!usage.windows[0].is_exhausted);
    assert!(
        usage
            .windows
            .iter()
            .find(|w| w.id == "model.primary")
            .unwrap()
            .is_exhausted
    );
    assert!(
        !usage
            .windows
            .iter()
            .find(|w| w.id == "model.secondary")
            .unwrap()
            .is_exhausted
    );
    assert!(matches!(usage.state, State::Live));
}

#[test]
fn ordinary_usage_refusal_is_authoritative_and_legacy_limits_still_decode() {
    let usage = parse_app_server_response(&json!({"ordinaryUsageAllowed":false,"rateLimits":{
        "planType":"plus","primary":{"usedPercent":2,"windowDurationMins":300,"resetsAt":1800000000},
        "secondary":{"usedPercent":1,"windowDurationMins":10080}}}));
    assert!(usage.windows[0].is_exhausted);
    assert!(!usage.windows[1].is_exhausted);
    assert_eq!(usage.windows[0].resets_at, Some(1800000000000));
    assert_eq!(usage.plan.as_deref(), Some("plus"));
}
