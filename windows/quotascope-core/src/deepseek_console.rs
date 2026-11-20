//! DeepSeek console protocol, independent of browser, secrets and network.
//! Based on Pulse 3696a65's console shapes.
use crate::model::Unavailability;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const ORIGIN: &str = "https://platform.deepseek.com";
pub const STORAGE_KEY: &str = "userToken";

pub fn storage_token(values: &BTreeMap<String, String>) -> Option<String> {
    let wrapper: Value = serde_json::from_str(values.get(STORAGE_KEY)?).ok()?;
    let token = wrapper.get("value")?.as_str()?.trim();
    (!token.is_empty() && token.len() <= 16_384 && token.bytes().all(|c| c.is_ascii_graphic()))
        .then(|| token.to_string())
}

fn code_problem(code: i64) -> Unavailability {
    if (40_000..40_100).contains(&code) {
        Unavailability::SessionExpired
    } else {
        Unavailability::ServerError
    }
}

/// Both envelope layers must affirm success. HTTP 200 alone is insufficient.
pub fn body(reply: &Value) -> Result<&Value, Unavailability> {
    let code = reply["code"]
        .as_i64()
        .ok_or(Unavailability::UnreadableReply)?;
    if code != 0 {
        return Err(code_problem(code));
    }
    let code = reply["data"]["biz_code"]
        .as_i64()
        .ok_or(Unavailability::UnreadableReply)?;
    if code != 0 {
        return Err(code_problem(code));
    }
    reply["data"]["biz_data"]
        .as_object()
        .map(|_| &reply["data"]["biz_data"])
        .ok_or(Unavailability::UnreadableReply)
}

fn figure(value: &Value) -> Option<f64> {
    crate::http::number(value).filter(|n| n.is_finite() && n.abs() < 1e15)
}

fn currency(value: &Value) -> Option<String> {
    let code = value.as_str()?.trim();
    (!code.is_empty() && code.len() <= 8 && code.bytes().all(|c| c.is_ascii_alphanumeric()))
        .then(|| code.to_ascii_uppercase())
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], Unavailability> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or(Unavailability::UnreadableReply)
}

/// Convert readable wallets into the API-key balance shape, without an
/// availability verdict or the console's token-estimation guesses.
pub fn parse_summary(reply: &Value) -> Result<Value, Unavailability> {
    let summary = body(reply)?;
    let mut order = Vec::<String>::new();
    let mut wallets = BTreeMap::<String, [Option<f64>; 2]>::new();
    for (index, key) in ["normal_wallets", "bonus_wallets"].iter().enumerate() {
        for wallet in array(summary, key)? {
            let (Some(code), Some(balance)) =
                (currency(&wallet["currency"]), figure(&wallet["balance"]))
            else {
                continue;
            };
            if balance < 0.0 {
                continue;
            }
            if !wallets.contains_key(&code) {
                order.push(code.clone());
            }
            let entries = wallets.entry(code).or_default();
            let amount = entries[index].get_or_insert(0.0);
            *amount += balance;
            if !amount.is_finite() {
                return Err(Unavailability::UnreadableReply);
            }
        }
    }
    let infos: Vec<_> = order
        .into_iter()
        .map(|code| {
            let pair = wallets[&code];
            json!({
                "currency":code,
                "total_balance":format!("{:.2}",pair[0].unwrap_or(0.0)+pair[1].unwrap_or(0.0)),
                "topped_up_balance":pair[0].map(|n|format!("{n:.2}")),
                "granted_balance":pair[1].map(|n|format!("{n:.2}")),
            })
        })
        .collect();
    Ok(json!({"balance_infos":infos}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(body: Value) -> Value {
        json!({"code":0,"data":{"biz_code":0,"biz_data":body}})
    }

    #[test]
    fn storage_wrapper_is_separate_from_api_keys_and_signed_out_is_absent() {
        let token =
            |value: &str| storage_token(&BTreeMap::from([(STORAGE_KEY.into(), value.into())]));
        assert_eq!(
            token(r#"{"value":" abc123 ","__version":"0"}"#).as_deref(),
            Some("abc123")
        );
        for value in [
            r#"{"value":null}"#,
            r#"{"value":" "}"#,
            r#"{"value":"line\nbreak"}"#,
            "abc123",
            r#"{"accessToken":"other"}"#,
        ] {
            assert!(token(value).is_none());
        }
        assert!(storage_token(&BTreeMap::new()).is_none());
    }

    #[test]
    fn envelope_checks_both_codes_and_requires_explicit_success() {
        for code in [40000, 40002, 40003, 40099] {
            assert_eq!(
                body(&json!({"code":code})),
                Err(Unavailability::SessionExpired)
            );
            assert_eq!(
                body(&json!({"code":0,"data":{"biz_code":code}})),
                Err(Unavailability::SessionExpired)
            );
        }
        for code in [1, 39999, 40100, 50000] {
            assert_eq!(
                body(&json!({"code":code})),
                Err(Unavailability::ServerError)
            );
        }
        for value in [
            json!({}),
            json!({"code":0}),
            json!({"code":0,"data":{"biz_data":{}}}),
            json!({"code":"0"}),
            reply(Value::Null),
        ] {
            assert_eq!(body(&value), Err(Unavailability::UnreadableReply));
        }
        assert_eq!(
            body(&reply(json!({"series":[]}))).unwrap(),
            &json!({"series":[]})
        );
    }

    #[test]
    fn normal_and_bonus_wallets_join_only_within_their_currency() {
        let summary = reply(json!({
            "normal_wallets":[{"balance":"8.15","currency":"CNY","token_estimation":"1"}],
            "bonus_wallets":[{"balance":"5.97","currency":"CNY"},{"balance":"1","currency":"USD"},{"balance":null,"currency":"EUR"},{"balance":-1,"currency":"JPY"}],
            "total_costs":[{"currency":"CNY","amount":"132.97"}]
        }));
        let result = parse_summary(&summary).unwrap();
        assert!(result.get("is_available").is_none());
        assert_eq!(result["balance_infos"].as_array().unwrap().len(), 2);
        assert_eq!(result["balance_infos"][0]["total_balance"], "14.12");
        assert_eq!(result["balance_infos"][1]["total_balance"], "1.00");
        assert!(result["balance_infos"][1]["topped_up_balance"].is_null());
        assert!(parse_summary(&reply(json!({}))).is_err());
        let empty = parse_summary(&reply(json!({"normal_wallets":[],"bonus_wallets":[]}))).unwrap();
        assert_eq!(empty["balance_infos"], json!([]));
    }
}
