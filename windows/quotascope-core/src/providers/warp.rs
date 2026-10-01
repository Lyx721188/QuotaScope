//! Warp: the plan's credits for the period, and any add-on credits bought or
//! granted on top.
//!
//! Read with a key the user pastes, from the GraphQL query Warp's own app
//! sends — `GetRequestLimitInfo`, a read-only query posted to
//! `https://app.warp.dev/graphql/v2`. The API still calls credits
//! "requests". The shape is second-hand — taken from CodexBar's Warp provider
//! and its tests, not from a captured reply — and the fixture in the tests
//! says so.
//!
//! - The plan's credits: `requestsUsedSinceLastRefresh` of `requestLimit`,
//!   both stated, refilling at `nextRefreshTime`. The period's length is not
//!   stated, so it is not claimed. An unlimited plan has no limit to draw.
//! - Add-on credits: each grant states what it was and what is left, so the
//!   pack is their sum, spent after the plan's and never reset; the soonest
//!   of them to lapse is shown as an expiry.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{HttpClient, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, State, Unavailability, UsageWindow};
use serde::Deserialize;
use std::sync::Arc;

const ENDPOINT: &str = "https://app.warp.dev/graphql/v2?op=GetRequestLimitInfo";

/// The OS version the request carries. Upstream mirrors the running OS's
/// version here — metadata the reply never reflects, and on Windows there is
/// no cheap reader for it — so the port carries a fixed placeholder under
/// upstream's own client labels.
const OS_VERSION: &str = "1.0.0";

/// The read-only query Warp's own app sends.
const QUERY: &str = r#"query GetRequestLimitInfo($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    __typename
    ... on UserOutput {
      user {
        requestLimitInfo { isUnlimited nextRefreshTime requestLimit requestsUsedSinceLastRefresh }
        bonusGrants { requestCreditsGranted requestCreditsRemaining expiration }
        workspaces { bonusGrantsInfo { grants { requestCreditsGranted requestCreditsRemaining expiration } } }
      }
    }
  }
}"#;

pub struct WarpService {
    http: Arc<HttpClient>,
}

impl WarpService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for WarpService {
    fn provider(&self) -> Provider {
        Provider::Warp
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Warp);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Warp)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
            // What Warp's app sends. The endpoint's edge limiter answers 429
            // to a client that does not name itself as Warp.
            ("x-warp-client-id", "warp-app".into()),
            ("x-warp-os-category", "macOS".into()),
            ("x-warp-os-name", "macOS".into()),
            ("x-warp-os-version", OS_VERSION.into()),
            ("User-Agent", "Warp/1.0".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let body = request_body();
        let reply = match self
            .http
            .fetch_json(Method::Post, ENDPOINT, &refs, Some(&body))
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = reading(&reply, account);
        if matches!(usage.state, State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

/// The GraphQL request exactly as upstream posts it.
pub fn request_body() -> serde_json::Value {
    serde_json::json!({
        "operationName": "GetRequestLimitInfo",
        "query": QUERY,
        "variables": {
            "requestContext": {
                "clientContext": {},
                "osContext": {"category": "macOS", "name": "macOS", "version": OS_VERSION},
            },
        },
    })
}

#[derive(Deserialize)]
struct Grant {
    #[serde(rename = "requestCreditsGranted")]
    request_credits_granted: Option<f64>,
    #[serde(rename = "requestCreditsRemaining")]
    request_credits_remaining: Option<f64>,
    expiration: Option<String>,
}

#[derive(Deserialize)]
struct Limit {
    #[serde(rename = "isUnlimited")]
    is_unlimited: Option<bool>,
    #[serde(rename = "nextRefreshTime")]
    next_refresh_time: Option<String>,
    #[serde(rename = "requestLimit")]
    request_limit: Option<f64>,
    #[serde(rename = "requestsUsedSinceLastRefresh")]
    requests_used_since_last_refresh: Option<f64>,
}

#[derive(Deserialize)]
struct GrantsInfo {
    grants: Option<Vec<Grant>>,
}

#[derive(Deserialize)]
struct Workspace {
    #[serde(rename = "bonusGrantsInfo")]
    bonus_grants_info: Option<GrantsInfo>,
}

#[derive(Deserialize)]
struct Account {
    #[serde(rename = "requestLimitInfo")]
    request_limit_info: Option<Limit>,
    #[serde(rename = "bonusGrants")]
    bonus_grants: Option<Vec<Grant>>,
    workspaces: Option<Vec<Workspace>>,
}

#[derive(Deserialize)]
struct Output {
    user: Option<Account>,
}

#[derive(Deserialize)]
struct Payload {
    user: Option<Output>,
}

#[derive(Deserialize)]
struct Reply {
    data: Option<Payload>,
    errors: Option<Vec<Failure>>,
}

#[derive(Deserialize)]
struct Failure {
    #[allow(dead_code)]
    message: Option<String>,
}

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    reading_at(reply, account, crate::timeutil::now_ms())
}

pub fn reading_at(reply: &serde_json::Value, account: AccountKey, now_ms: i64) -> ProviderUsage {
    let Ok(reply) = serde_json::from_value::<Reply>(reply.clone()) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    // GraphQL answers a failed query with 200 and a list of errors.
    if reply
        .errors
        .as_ref()
        .is_some_and(|errors| !errors.is_empty())
    {
        return ProviderUsage::unavailable(account, Unavailability::ServerError);
    }
    let Some(reply_account) = reply
        .data
        .and_then(|payload| payload.user)
        .and_then(|output| output.user)
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let Some(limit) = reply_account.request_limit_info else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    let mut windows: Vec<UsageWindow> = Vec::new();
    if limit.is_unlimited != Some(true) {
        if let (Some(cap), Some(used)) =
            (limit.request_limit, limit.requests_used_since_last_refresh)
        {
            if cap.is_finite() && cap > 0.0 && used.is_finite() && used >= 0.0 {
                let mut row = UsageWindow::new(
                    "warp.credits",
                    Kind::Credits,
                    None,
                    used / cap,
                    30 * 86_400,
                    limit
                        .next_refresh_time
                        .as_deref()
                        .and_then(crate::timeutil::parse_iso8601_ms),
                );
                // A sort key: the refill is stated, the period's length is
                // not.
                row.reports_length = false;
                row.is_exhausted = used >= cap;
                windows.push(row);
            }
        }
    }

    // Every grant with both figures, the user's own and each workspace's.
    let grants: Vec<(f64, f64, Option<i64>)> = reply_account
        .bonus_grants
        .unwrap_or_default()
        .into_iter()
        .chain(
            reply_account
                .workspaces
                .unwrap_or_default()
                .into_iter()
                .filter_map(|workspace| workspace.bonus_grants_info)
                .filter_map(|info| info.grants)
                .flatten(),
        )
        .filter_map(|grant| {
            let (Some(granted), Some(left)) = (
                grant.request_credits_granted,
                grant.request_credits_remaining,
            ) else {
                return None;
            };
            if !(granted.is_finite() && granted > 0.0 && left.is_finite() && left >= 0.0) {
                return None;
            }
            let expires = grant
                .expiration
                .as_deref()
                .and_then(crate::timeutil::parse_iso8601_ms);
            Some((granted, left, expires))
        })
        .collect();

    let granted: f64 = grants.iter().map(|(granted, _, _)| granted).sum();
    if granted > 0.0 {
        let left: f64 = grants.iter().map(|(_, left, _)| left).sum();
        let mut pack = UsageWindow::new(
            "warp.addon",
            Kind::TopUp,
            None,
            (granted - left).max(0.0) / granted,
            30 * 86_400,
            None,
        );
        // A sort key alone: spent after the plan's, and never reset.
        pack.reports_length = false;
        pack.is_exhausted = left <= 0.0;
        pack.set_expiring_parts(
            grants
                .iter()
                .filter_map(|(_, left, expiry)| expiry.map(|at| (*left, at))),
            now_ms,
        );
        windows.push(pack);
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    ProviderUsage::live_now(account, windows)
}

/// The soonest a part of the pack stops existing: the earliest expiry still
/// ahead among grants that have credits left. Upstream also sums the parts
/// lapsing that same day; the local window carries the time alone.
pub fn next_expiry(grants: &[(f64, f64, Option<i64>)], now_ms: i64) -> Option<i64> {
    grants
        .iter()
        .filter_map(|(_, left, expires)| expires.filter(|at| *at > now_ms && *left > 0.0))
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    const NOW: i64 = 1_790_812_800_000; // 2026-10-01T00:00:00Z

    fn reading_at_for(value: serde_json::Value) -> ProviderUsage {
        reading_at(&value, AccountKey::primary(Provider::Warp), NOW)
    }

    fn live_windows(usage: &ProviderUsage) -> &[UsageWindow] {
        match &usage.state {
            State::Live => &usage.windows,
            other => panic!("expected a live reading, got {other:?}"),
        }
    }

    fn reply_with(limit: serde_json::Value) -> serde_json::Value {
        json!({"data": {"user": {"user": {"requestLimitInfo": limit}}}})
    }

    #[test]
    fn the_plan_credits_read_as_one_window() {
        let usage = reading_at_for(reply_with(json!({
            "isUnlimited": false,
            "nextRefreshTime": "2026-10-05T00:00:00Z",
            "requestLimit": 500,
            "requestsUsedSinceLastRefresh": 125,
        })));
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        let window = &windows[0];
        assert_eq!(window.id, "warp.credits");
        assert_eq!(window.kind, Kind::Credits);
        assert!((window.used_fraction - 0.25).abs() < 1e-9);
        // The refill is stated, the period's length is not.
        assert!(!window.reports_length);
        assert_eq!(
            window.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-05T00:00:00Z")
        );
        assert!(!window.is_exhausted);
    }

    #[test]
    fn an_unlimited_plan_reports_no_limits() {
        // No limit to draw and no grants on top: there is nothing to show,
        // which upstream says outright rather than drawing an empty card.
        let usage = reading_at_for(reply_with(json!({
            "isUnlimited": true,
            "requestLimit": 500,
            "requestsUsedSinceLastRefresh": 125,
        })));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn a_limit_without_both_figures_is_not_drawn() {
        for limit in [
            json!({"isUnlimited": false, "requestLimit": 500}),
            json!({"isUnlimited": false, "requestsUsedSinceLastRefresh": 10}),
            json!({"isUnlimited": false, "requestLimit": 0, "requestsUsedSinceLastRefresh": 0}),
        ] {
            let usage = reading_at_for(reply_with(limit));
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::NoLimitsReported)
            ));
        }
    }

    #[test]
    fn a_failed_query_answers_with_errors_under_a_200() {
        let usage = reading_at_for(json!({"data": null, "errors": [{"message": "Unauthorized"}]}));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::ServerError)
        ));
    }

    #[test]
    fn a_reply_without_an_account_is_unreadable() {
        for reply in [
            json!({}),
            json!({"data": {"user": {"user": null}}}),
            json!({"data": {"user": {"user": {}}}}),
            // The figures are numbers; strings fail the decode the way
            // Swift's would.
            json!({"data": {"user": {"user": {"requestLimitInfo": {"requestLimit": "500"}}}}}),
        ] {
            let usage = reading_at_for(reply);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn grants_sum_across_the_account_and_its_workspaces() {
        let mut reply = reply_with(json!({
            "isUnlimited": false,
            "requestLimit": 500,
            "requestsUsedSinceLastRefresh": 0,
        }));
        reply["data"]["user"]["user"]["bonusGrants"] = json!([
            {"requestCreditsGranted": 100, "requestCreditsRemaining": 60, "expiration": "2026-10-03T00:00:00Z"},
            {"requestCreditsGranted": 50, "requestCreditsRemaining": 0},
        ]);
        reply["data"]["user"]["user"]["workspaces"] = json!([
            {"bonusGrantsInfo": {"grants": [
                {"requestCreditsGranted": 200, "requestCreditsRemaining": 140, "expiration": "2026-10-02T00:00:00Z"},
            ]}},
        ]);
        let usage = reading_at_for(reply);
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 2);
        let pack = &windows[1];
        assert_eq!(pack.id, "warp.addon");
        assert_eq!(pack.kind, Kind::TopUp);
        assert!((pack.used_fraction - 150.0 / 350.0).abs() < 1e-9);
        assert!(!pack.reports_length);
        assert!(pack.resets_at.is_none());
        assert!(!pack.is_exhausted);
        // The soonest of the grants still holding credits to lapse.
        assert_eq!(
            pack.next_expiry_ms,
            crate::timeutil::parse_iso8601_ms("2026-10-02T00:00:00Z")
        );
    }

    #[test]
    fn an_empty_pack_reads_as_spent() {
        let mut reply = reply_with(json!({"isUnlimited": true}));
        reply["data"]["user"]["user"]["bonusGrants"] = json!([
            {"requestCreditsGranted": 100, "requestCreditsRemaining": 0},
        ]);
        let usage = reading_at_for(reply);
        let pack = &live_windows(&usage)[0];
        assert!(pack.is_exhausted);
        assert_eq!(pack.used_fraction, 1.0);
    }

    #[test]
    fn the_expiry_skips_spent_grants_and_times_gone_by() {
        let grants = vec![
            (100.0, 0.0, Some(NOW + 3_600_000)),
            (100.0, 50.0, Some(NOW - 3_600_000)),
            (100.0, 50.0, None),
            (100.0, 20.0, Some(NOW + 86_400_000)),
        ];
        assert_eq!(next_expiry(&grants, NOW), Some(NOW + 86_400_000));
        assert_eq!(next_expiry(&grants[..1], NOW), None);
    }

    #[test]
    fn grants_without_both_figures_do_not_count() {
        let mut reply = reply_with(json!({"isUnlimited": true}));
        reply["data"]["user"]["user"]["bonusGrants"] = json!([
            {"requestCreditsRemaining": 60},
            {"requestCreditsGranted": 100},
            {"requestCreditsGranted": 0, "requestCreditsRemaining": 0},
        ]);
        let usage = reading_at_for(reply);
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn the_request_body_carries_the_named_operation() {
        let body = request_body();
        assert_eq!(body["operationName"], "GetRequestLimitInfo");
        assert!(body["query"]
            .as_str()
            .unwrap()
            .contains("GetRequestLimitInfo"));
        assert_eq!(
            body["variables"]["requestContext"]["osContext"]["category"],
            "macOS"
        );
    }
}
