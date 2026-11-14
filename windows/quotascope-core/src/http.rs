//! Shared HTTP plumbing. Every provider rides the same client — system
//! proxy settings included — and maps status codes to the same
//! unavailability vocabulary, with the same two-retry rule for connections
//! that stumble rather than refuse.

use crate::model::Unavailability;
use reqwest::blocking::{Client, ClientBuilder, Response};
use std::time::Duration;

pub struct HttpClient {
    client: Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl HttpClient {
    pub fn new() -> Self {
        let client = ClientBuilder::new()
            // URLSession's default, and the same figure the Swift services
            // set per request.
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(15))
            .user_agent("QuotaScope/1.1 (Windows)")
            // Provider credentials must never follow a redirect to another
            // host. A 3xx is classified below as a refused credential.
            .redirect(reqwest::redirect::Policy::none())
            // System proxy settings apply, as on macOS: a machine behind a
            // VPN or corporate proxy reaches these endpoints through it.
            .use_native_tls()
            .build()
            .unwrap_or_else(|_| {
                ClientBuilder::new()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .expect("initialize the credential-safe HTTP client")
            });
        HttpClient { client }
    }

    /// Raw client access for the flows that drive requests by hand (the
    /// GitHub device login).
    pub fn client_for_login(&self) -> Client {
        crate::proxy::client(&self.client)
    }

    /// One call, with the retry rule the Swift services share: two extra
    /// attempts, 600ms apart and growing, for errors that look like the
    /// connection tripping rather than the service refusing.
    pub fn fetch_json(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, Unavailability> {
        self.fetch_json_detailed(method, url, headers, body)
            .map_err(|failure| match failure {
                HttpFailure::NotFound => Unavailability::ServerError,
                HttpFailure::Unavailable(reason) => reason,
            })
    }

    /// The same call, keeping 404 distinct — MiniMax's older endpoint path
    /// answers 404 on accounts that are not failures at all.
    pub fn fetch_json_detailed(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, HttpFailure> {
        let mut last_error = Unavailability::Unreachable;
        for attempt in 0..=2 {
            let result = self.fetch_once(method, url, headers, body);
            match result {
                Ok(value) => return Ok(value),
                Err(Outcome::Unavailable(reason)) => return Err(HttpFailure::Unavailable(reason)),
                Err(Outcome::NotFound) => return Err(HttpFailure::NotFound),
                Err(Outcome::Stumble(reason)) => {
                    last_error = reason;
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(600 * (attempt as u64 + 1)));
                    }
                }
            }
        }
        Err(HttpFailure::Unavailable(last_error))
    }

    fn fetch_once(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, Outcome> {
        let client = self.client_for_login();
        let mut request = match method {
            Method::Get => client.get(url),
            Method::Post => client.post(url),
        };
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(json) = body {
            request = request
                .header("Content-Type", "application/json")
                .json(json);
        }

        let response = match request.send() {
            Ok(r) => r,
            Err(_) => {
                crate::connection_diagnostics::record(
                    method_name(method),
                    url,
                    None,
                    "",
                    "Network request failed",
                    headers,
                    body,
                );
                return Err(Outcome::Stumble(Unavailability::Unreachable));
            }
        };

        let status = response.status();
        let text = response_text(response, method_name(method), url, headers, body)
            .map_err(|_| Outcome::Unavailable(Unavailability::UnreadableReply))?;
        if status.as_u16() == 404 {
            return Err(Outcome::NotFound);
        }
        if !status.is_success() {
            let reason = match status.as_u16() {
                403 if url == "https://opencode.ai/zen/go/v1/usage"
                    && serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|v| {
                            v.pointer("/error/type")
                                .and_then(|v| v.as_str())
                                .map(str::to_owned)
                        })
                        .as_deref()
                        == Some("EntitlementError") =>
                {
                    Unavailability::NoPlan
                }
                401 | 403 => Unavailability::ApiKeyRefused,
                300..=399 => Unavailability::ApiKeyRefused,
                429 => Unavailability::RateLimited,
                _ => Unavailability::ServerError,
            };
            // A 401 from an OAuth endpoint means credentials, not a pasted
            // key; callers refine it, and an Unavailable here ends the call.
            return Err(Outcome::Unavailable(reason));
        }

        serde_json::from_str(&text)
            .map_err(|_| Outcome::Unavailable(Unavailability::UnreadableReply))
    }
}

enum Outcome {
    /// The service answered and the answer is no. Do not retry.
    Unavailable(Unavailability),
    /// A connection stumble; worth a second attempt.
    Stumble(Unavailability),
    /// 404 — some providers treat an older path's absence as a signal.
    NotFound,
}

pub enum HttpFailure {
    NotFound,
    Unavailable(Unavailability),
}

/// Response post-processing shared by the callers that need the status
/// separately from the body (GitHub device flow answers 200 while it waits).
pub fn status_of(response: &Response) -> u16 {
    response.status().as_u16()
}

/// A number that arrived as a string or as a number, interchangeably — the
/// MiniMax rule, and several others drifted into the same shape.
pub fn number(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

pub fn string_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(|v| v.as_str())
}

pub fn number_field(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key).and_then(number)
}

pub fn bool_field(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|v| v.as_bool())
}

pub fn array_field<'a>(value: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[])
}

pub fn object_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    value.get(key).filter(|v| v.is_object())
}

fn method_name(method: Method) -> &'static str {
    match method {
        Method::Get => "GET",
        Method::Post => "POST",
    }
}
pub fn response_text(
    response: Response,
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&serde_json::Value>,
) -> Result<String, Unavailability> {
    use std::io::Read;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut bytes = Vec::new();
    response
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Unavailability::UnreadableReply)?;
    if bytes.len() > 4 * 1024 * 1024 {
        crate::connection_diagnostics::record(
            method,
            url,
            Some(status),
            &content_type,
            "[Response exceeds 4 MiB limit]",
            headers,
            body,
        );
        return Err(Unavailability::UnreadableReply);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    crate::connection_diagnostics::record(
        method,
        url,
        Some(status),
        &content_type,
        &text,
        headers,
        body,
    );
    Ok(text)
}

/// Provider-specific routes retain metadata and bytes while sending a masked
/// copy to a user-initiated diagnostics capture.
pub trait ObserveRequest {
    fn observed_send(self) -> Result<ObservedResponse, Unavailability>;
}
impl ObserveRequest for reqwest::blocking::RequestBuilder {
    fn observed_send(self) -> Result<ObservedResponse, Unavailability> {
        use std::io::Read;
        let (client, request) = self.build_split();
        let request = request.map_err(|_| Unavailability::Unreachable)?;
        let method = request.method().as_str().to_string();
        let address = request.url().to_string();
        let headers: Vec<(String, String)> = request
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (key.to_string(), value.to_string()))
            })
            .collect();
        let refs: Vec<_> = headers
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let body = request
            .body()
            .and_then(|body| body.as_bytes())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok());
        let mut inner = client.execute(request).map_err(|_| {
            crate::connection_diagnostics::record(
                &method,
                &address,
                None,
                "",
                "Network request failed",
                &refs,
                body.as_ref(),
            );
            Unavailability::Unreachable
        })?;
        let mut bytes = Vec::new();
        (&mut inner)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Unavailability::UnreadableReply)?;
        if bytes.len() > 16 * 1024 * 1024 {
            crate::connection_diagnostics::record(
                &method,
                &address,
                Some(inner.status().as_u16()),
                "",
                "[Response exceeds 16 MiB limit]",
                &refs,
                body.as_ref(),
            );
            return Err(Unavailability::UnreadableReply);
        }
        let content_type = inner
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        crate::connection_diagnostics::record(
            &method,
            &address,
            Some(inner.status().as_u16()),
            content_type,
            &String::from_utf8_lossy(&bytes),
            &refs,
            body.as_ref(),
        );
        Ok(ObservedResponse {
            inner,
            bytes: std::io::Cursor::new(bytes),
        })
    }
}
pub struct ObservedResponse {
    inner: Response,
    bytes: std::io::Cursor<Vec<u8>>,
}
impl std::ops::Deref for ObservedResponse {
    type Target = Response;
    fn deref(&self) -> &Response {
        &self.inner
    }
}
impl std::io::Read for ObservedResponse {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.bytes.read(buffer)
    }
}
impl ObservedResponse {
    pub fn text(self) -> Result<String, Unavailability> {
        Ok(String::from_utf8_lossy(self.bytes.get_ref()).into_owned())
    }
    pub fn json<T: serde::de::DeserializeOwned>(self) -> Result<T, Unavailability> {
        serde_json::from_slice(self.bytes.get_ref()).map_err(|_| Unavailability::UnreadableReply)
    }
}
