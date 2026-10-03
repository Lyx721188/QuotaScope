//! Windows equivalents of the remaining upstream routes. Saved client
//! credentials are read only; only provider-owned percentages become rings.
use super::{KeyRing, ProviderService, SessionSpec};
use crate::http::{number, HttpClient, Method};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

pub const OLLAMA_SESSION: SessionSpec = SessionSpec {
    hosts: &["ollama.com"],
    cookies: &[
        "wos-session",
        "__Secure-session",
        "__Secure-next-auth.session-token",
        "next-auth.session-token",
        "wos-session.*",
        "__Secure-session.*",
        "__Secure-next-auth.session-token.*",
        "next-auth.session-token.*",
    ],
};
pub const PROVIDERS: &[Provider] = &[
    Provider::Kiro,
    Provider::OllamaCloud,
    Provider::GrokBot,
    Provider::Volcengine,
    Provider::Devin,
    Provider::AlibabaTokenPlan,
    Provider::Gemini,
    Provider::JetBrainsAi,
    Provider::Windsurf,
    Provider::NousPortal,
];
pub struct Service {
    provider: Provider,
    http: Arc<HttpClient>,
}
impl Service {
    pub fn new(provider: Provider, http: Arc<HttpClient>) -> Self {
        Self { provider, http }
    }
}
impl ProviderService for Service {
    fn provider(&self) -> Provider {
        self.provider
    }
    fn origin_token(&self) -> &'static str {
        "localLogin"
    }
    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(self.provider);
        match self.fetch_inner(keys) {
            Ok(usage) => usage,
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}
impl Service {
    fn fetch_inner(&self, keys: &KeyRing) -> Result<ProviderUsage, Unavailability> {
        match self.provider {
            Provider::Gemini => {
                if let Some(settings) = json_file(crate::model::home_path(".gemini/settings.json"))
                {
                    let selected = settings
                        .pointer("/security/auth/selectedType")
                        .and_then(Value::as_str)
                        .or_else(|| settings["selectedAuthType"].as_str());
                    if matches!(selected, Some("gemini-api-key" | "api-key" | "vertex-ai")) {
                        return Err(Unavailability::LocalLoginMissing);
                    }
                }
                let root = json_file(crate::model::home_path(".gemini/oauth_creds.json"))
                    .ok_or(Unavailability::LocalLoginMissing)?;
                if number(&root["expiry_date"])
                    .is_some_and(|t| t <= crate::timeutil::now_ms() as f64)
                {
                    return Err(Unavailability::LocalLoginExpired);
                }
                let token = root["access_token"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .ok_or(Unavailability::LocalLoginExpired)?;
                let auth = format!("Bearer {token}");
                let assist = self
                    .http
                    .fetch_json(
                        Method::Post,
                        "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist",
                        &[("Authorization", &auth)],
                        Some(&json!({"metadata":{"ideType":"GEMINI_CLI","pluginType":"GEMINI"}})),
                    )
                    .map_err(login_error)?;
                let project = assist["cloudaicompanionProject"]
                    .as_str()
                    .or_else(|| assist["cloudaicompanionProject"]["id"].as_str());
                let body = project.map(|p| json!({"project":p})).unwrap_or(json!({}));
                let reply = self
                    .http
                    .fetch_json(
                        Method::Post,
                        "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota",
                        &[("Authorization", &auth)],
                        Some(&body),
                    )
                    .map_err(login_error)?;
                Ok(parse(self.provider, &reply, Some("endpoint")))
            }
            Provider::NousPortal => {
                let mut login = None;
                let mut expired = false;
                for file in [".hermes/auth.json", ".hermes/shared/nous_auth.json"] {
                    let Some(root) = json_file(crate::model::home_path(file)) else {
                        continue;
                    };
                    if let Some((token, expiry)) = nous_login(&root) {
                        if expiry.is_some_and(|t| t <= crate::timeutil::now_ms() + 60000) {
                            expired = true;
                            continue;
                        }
                        login = Some(token);
                        break;
                    }
                }
                let token = login.ok_or(if expired {
                    Unavailability::LocalLoginExpired
                } else {
                    Unavailability::LocalLoginMissing
                })?;
                // Fixed provider-owned host: no credential follows an arbitrary
                // portal URL from a client settings file.
                let reply = self
                    .http
                    .fetch_json(
                        Method::Get,
                        "https://portal.nousresearch.com/api/oauth/account",
                        &[("Authorization", &format!("Bearer {token}"))],
                        None,
                    )
                    .map_err(login_error)?;
                Ok(parse(self.provider, &reply, Some("endpoint")))
            }
            Provider::GrokBot => {
                let token =
                    super::cursor::stored_token().ok_or(Unavailability::CursorSignInRequired)?;
                let cookie = super::cursor::session_cookie(&token)
                    .ok_or(Unavailability::CursorLoginExpired)?;
                let reply = self.http.fetch_json(
                    Method::Post,
                    "https://cursor.com/api/dashboard/get-sand-usage-status",
                    &[("Cookie", &cookie)],
                    Some(&json!({})),
                )?;
                Ok(parse(self.provider, &reply, Some("endpoint")))
            }
            Provider::Devin => {
                let roaming = std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                let mut readings = Vec::new();
                for app in ["Devin", "Windsurf"] {
                    let path = roaming.join(app).join("User/globalStorage/state.vscdb");
                    let Ok(db) = rusqlite::Connection::open_with_flags(
                        &path,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                    ) else {
                        continue;
                    };
                    let Ok(mut q) = db.prepare("SELECT value FROM ItemTable WHERE key LIKE 'windsurf.reactSettings.cachedPlanInfoData%' OR key LIKE 'windsurf.settings.cachedPlanInfo%'") else { continue; };
                    if let Ok(rows) = q.query_map([], |r| r.get::<_, String>(0)) {
                        for row in rows.flatten() {
                            if let Ok(root) = serde_json::from_str::<Value>(&row) {
                                let mut usage = parse(self.provider, &root, Some("savedPlan"));
                                if matches!(usage.state, crate::model::State::Live) {
                                    usage.state = crate::model::State::Stale;
                                    usage.is_cached = true;
                                }
                                usage.observed_at = path
                                    .metadata()
                                    .and_then(|m| m.modified())
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_millis() as i64);
                                readings.push(usage);
                            }
                        }
                    };
                }
                readings
                    .into_iter()
                    .max_by_key(|u| u.observed_at)
                    .ok_or(Unavailability::LocalAppMissing)
            }
            Provider::JetBrainsAi => jetbrains(),
            Provider::AlibabaTokenPlan => {
                let mut error = Unavailability::LocalLoginMissing;
                for (site, region) in [
                    ("international", "ap-southeast-1"),
                    ("domestic", "cn-beijing"),
                ] {
                    match run_cli(
                        "bl",
                        &[
                            "usage",
                            "token-plan",
                            "--console-region",
                            region,
                            "--console-site",
                            site,
                            "--output",
                            "json",
                        ],
                        false,
                    ) {
                        Ok(root) => {
                            let usage = parse(self.provider, &root, Some("cli"));
                            if !usage.windows.is_empty() {
                                return Ok(usage);
                            }
                        }
                        Err(reason) => error = reason,
                    }
                }
                Err(error)
            }
            Provider::Volcengine => {
                if let Some(key) = keys.api_key(self.provider).filter(|k| !k.trim().is_empty()) {
                    return super::volcengine_signer::fetch(&self.http, &key);
                }
                Ok(parse(
                    self.provider,
                    &run_cli("arkcli", &["usage", "plan", "--format", "json"], false)?,
                    Some("cli"),
                ))
            }
            Provider::Kiro => Ok(parse(
                self.provider,
                &run_cli(
                    "kiro-cli",
                    &["acp", "--agent-engine", "v3", "--auth-method", "cli"],
                    true,
                )?,
                Some("acp"),
            )),
            Provider::OllamaCloud => self.ollama(keys),
            Provider::Windsurf => self.windsurf(keys),
            _ => Err(Unavailability::NoLimitsReported),
        }
    }

    fn ollama(&self, keys: &KeyRing) -> Result<ProviderUsage, Unavailability> {
        let cookie = keys
            .api_key(self.provider)
            .filter(|c| !c.is_empty())
            .ok_or(Unavailability::SessionMissing)?;
        let cookie = ollama_cookie(&cookie).ok_or(Unavailability::SessionMissing)?;
        let reply = self
            .http
            .client_for_login()
            .get("https://ollama.com/settings")
            .header("Cookie", cookie)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        if !reply.status().is_success() {
            return Err(match reply.status().as_u16() {
                401 | 403 => Unavailability::SessionExpired,
                429 => Unavailability::RateLimited,
                _ => Unavailability::ServerError,
            });
        }
        let mut bytes = Vec::new();
        reply
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Unavailability::UnreadableReply)?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(Unavailability::UnreadableReply);
        }
        let text = String::from_utf8(bytes).map_err(|_| Unavailability::UnreadableReply)?;
        let mut windows = Vec::new();
        for (label, id, kind, seconds) in [
            ("Session usage", "ollama.session", Kind::FiveHour, 18000),
            ("Weekly usage", "ollama.weekly", Kind::Weekly, 604800),
        ] {
            let (_, section) = text
                .split_once(label)
                .ok_or(Unavailability::UnreadableReply)?;
            let section = section
                .split(if id.ends_with("session") {
                    "Weekly usage"
                } else {
                    "Session usage"
                })
                .next()
                .unwrap_or(section);
            let Some(percent) = percent_text(section) else {
                return Err(Unavailability::UnreadableReply);
            };
            let reset = section
                .split_once("data-time=\"")
                .and_then(|(_, s)| s.split('"').next())
                .and_then(crate::timeutil::parse_iso8601_ms);
            windows.push(UsageWindow::new(
                id,
                kind,
                None,
                percent / 100.0,
                seconds,
                reset,
            ));
        }
        let mut usage = ProviderUsage::live_now(AccountKey::primary(self.provider), windows);
        usage.origin = Some("webSession".into());
        Ok(usage)
    }

    fn windsurf(&self, keys: &KeyRing) -> Result<ProviderUsage, Unavailability> {
        let key = keys
            .api_key(self.provider)
            .ok_or(Unavailability::SessionMissing)?;
        let session: Value =
            serde_json::from_str(&key).map_err(|_| Unavailability::SessionMissing)?;
        let mut request = self.http.client_for_login().post("https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/GetPlanStatus").header("Content-Type","application/proto").header("Connect-Protocol-Version","1").header("Origin","https://windsurf.com").header("Referer","https://windsurf.com/profile");
        let token = session["devin_session_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Unavailability::SessionMissing)?;
        for (field, key) in [
            ("x-auth-token", "devin_session_token"),
            ("x-devin-session-token", "devin_session_token"),
            ("x-devin-auth1-token", "devin_auth1_token"),
            ("x-devin-account-id", "devin_account_id"),
            ("x-devin-primary-org-id", "devin_primary_org_id"),
        ] {
            request = request.header(
                field,
                session[key]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(Unavailability::SessionMissing)?,
            );
        }
        let mut body = vec![10];
        varint(token.len() as u64, &mut body);
        body.extend(token.as_bytes());
        body.extend([16, 1]);
        let response = request
            .body(body)
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        if !response.status().is_success() {
            return Err(Unavailability::SessionExpired);
        }
        let mut bytes = Vec::new();
        response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Unavailability::UnreadableReply)?;
        if bytes.len() > 1024 * 1024 {
            return Err(Unavailability::UnreadableReply);
        }
        let status = proto(&bytes)?
            .into_iter()
            .find_map(|(n, v)| {
                if n == 1 {
                    if let Proto::Bytes(v) = v {
                        Some(v)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .ok_or(Unavailability::UnreadableReply)?;
        let fields = proto(&status)?;
        let numeric = |number| {
            fields.iter().find_map(|(n, v)| {
                if *n == number {
                    if let Proto::Int(v) = v {
                        Some(*v)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        };
        let mut windows = Vec::new();
        for (id, kind, seconds, remaining, reset) in [
            ("daily", Kind::Daily, 86400, 14, 17),
            ("weekly", Kind::Weekly, 604800, 15, 18),
        ] {
            if let Some(left) = numeric(remaining).filter(|v| *v <= 100) {
                windows.push(UsageWindow::new(
                    id,
                    kind,
                    None,
                    1.0 - left as f64 / 100.0,
                    seconds,
                    numeric(reset)
                        .and_then(|v| i64::try_from(v).ok())
                        .and_then(|v| v.checked_mul(1000)),
                ));
            }
        }
        Ok(reading(self.provider, windows, None, "webSession"))
    }
}

fn json_file(path: PathBuf) -> Option<Value> {
    let input = std::fs::File::open(path).ok()?;
    let mut data = Vec::new();
    input.take(1024 * 1024).read_to_end(&mut data).ok()?;
    serde_json::from_slice(&data).ok()
}
fn login_error(reason: Unavailability) -> Unavailability {
    if reason == Unavailability::ApiKeyRefused {
        Unavailability::LocalLoginExpired
    } else {
        reason
    }
}
fn nous_login(root: &Value) -> Option<(String, Option<i64>)> {
    let entry = |state: &Value| {
        let token = state["access_token"].as_str()?.trim();
        if token.is_empty() {
            return None;
        }
        let jwt_expiry = || {
            use base64::Engine;
            let part = token.split('.').nth(1)?;
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(part.trim_end_matches('='))
                .ok()?;
            let claims: Value = serde_json::from_slice(&bytes).ok()?;
            claims["exp"].as_i64()?.checked_mul(1000)
        };
        Some((
            token.to_string(),
            stamp(&state["expires_at"]).or_else(jwt_expiry),
        ))
    };
    entry(&root["providers"]["nous"])
        .or_else(|| {
            root["credential_pool"]["nous"]
                .as_array()?
                .iter()
                .filter_map(entry)
                .max_by_key(|(_, expiry)| *expiry)
        })
        .or_else(|| entry(root))
}
pub(crate) fn stamp(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(crate::timeutil::parse_iso8601_ms)
        .or_else(|| {
            number(value)
                .filter(|n| n.is_finite() && *n > 0.0)
                .map(|n| (n * if n < 1e11 { 1000.0 } else { 1.0 }) as i64)
        })
}
fn num(value: &Value) -> Option<f64> {
    number(value).filter(|n| n.is_finite() && *n >= 0.0)
}
pub(super) fn reading(
    provider: Provider,
    windows: Vec<UsageWindow>,
    plan: Option<String>,
    origin: &str,
) -> ProviderUsage {
    if windows.is_empty() {
        return ProviderUsage::unavailable(
            AccountKey::primary(provider),
            Unavailability::NoLimitsReported,
        );
    }
    let mut usage = ProviderUsage::live_now(AccountKey::primary(provider), windows);
    usage.plan = plan;
    usage.origin = Some(origin.into());
    usage
}
fn locate(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<_> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    dirs.extend([
        crate::home_dir().join(".local/bin"),
        crate::home_dir().join(".bun/bin"),
    ]);
    dirs.into_iter()
        .flat_map(|dir| {
            [
                dir.join(format!("{name}.exe")),
                dir.join(format!("{name}.cmd")),
                dir.join(name),
            ]
        })
        .find(|p| p.is_file())
}
fn run_cli(name: &str, args: &[&str], acp: bool) -> Result<Value, Unavailability> {
    use std::process::{Command, Stdio};
    let path = locate(name).ok_or(Unavailability::LocalLoginMissing)?;
    if acp {
        return run_acp(path, args);
    }
    let mut cmd = if path.extension().is_some_and(|e| e == "cmd") {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/d", "/c"]).arg(&path);
        cmd
    } else {
        Command::new(&path)
    };
    cmd.args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "SystemRoot",
        "PATH",
        "TEMP",
        "TMP",
        "HOMEDRIVE",
        "HOMEPATH",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
    ] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    crate::proxy::environment(&mut cmd);
    let mut child = cmd.spawn().map_err(|_| Unavailability::LocalLoginMissing)?;
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let error = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let result = output.take(512 * 1024 + 1).read_to_end(&mut data);
        let _ = tx.send((result.is_ok(), data));
    });
    let (error_tx, error_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let _ = error.take(64 * 1024).read_to_end(&mut data);
        let _ = error_tx.send(data);
    });
    let deadline = std::time::Instant::now();
    let mut result = None;
    drop(input);
    loop {
        if let Ok((ok, data)) = rx.try_recv() {
            if !ok || data.len() > 512 * 1024 {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Unavailability::UnreadableReply);
            }
            result = Some(data);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let data = error_rx
                        .recv_timeout(std::time::Duration::from_millis(250))
                        .unwrap_or_default();
                    let text = String::from_utf8_lossy(&data).to_lowercase();
                    return Err(
                        if [
                            "not logged in",
                            "please login",
                            "please log in",
                            "login expired",
                            "unauthorized",
                        ]
                        .iter()
                        .any(|s| text.contains(s))
                        {
                            Unavailability::LocalLoginExpired
                        } else {
                            Unavailability::UnreadableReply
                        },
                    );
                }
                break;
            }
            Err(_) => return Err(Unavailability::Unreachable),
            _ => {}
        }
        if deadline.elapsed().as_secs() >= 15 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Unavailability::Unreachable);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let bytes = result
        .or_else(|| {
            rx.recv_timeout(std::time::Duration::from_secs(1))
                .ok()
                .filter(|(ok, _)| *ok)
                .map(|(_, data)| data)
        })
        .ok_or(Unavailability::UnreadableReply)?;
    if bytes.len() > 512 * 1024 {
        return Err(Unavailability::UnreadableReply);
    }
    serde_json::from_slice(&bytes).map_err(|_| Unavailability::UnreadableReply)
}
fn run_acp(path: PathBuf, args: &[&str]) -> Result<Value, Unavailability> {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    let mut command = Command::new(path);
    command.env_clear();
    for key in [
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "SystemRoot",
        "PATH",
        "TEMP",
        "TMP",
        "HOMEDRIVE",
        "HOMEPATH",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    crate::proxy::environment(&mut command);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| Unavailability::LocalLoginMissing)?;
    let mut stdin = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    });
    let (tx, rx) = std::sync::mpsc::sync_channel(32);
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(output.take(512 * 1024))
            .lines()
            .map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let result = (|| {
        for (id, method, params) in [
            (
                1,
                "initialize",
                json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"QuotaScope","version":"1.2"}}),
            ),
            (2, "_kiro/account/getUsage", json!({})),
        ] {
            writeln!(
                stdin,
                "{}",
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
            )
            .map_err(|_| Unavailability::Unreachable)?;
            let start = std::time::Instant::now();
            loop {
                let timeout = std::time::Duration::from_secs(20).saturating_sub(start.elapsed());
                let line = rx
                    .recv_timeout(timeout)
                    .map_err(|_| Unavailability::Unreachable)?;
                let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if reply["id"].as_i64() != Some(id) {
                    continue;
                }
                if !reply["error"].is_null() {
                    return Err(Unavailability::LocalLoginExpired);
                }
                if id == 2 {
                    return Ok(reply["result"].clone());
                }
                break;
            }
        }
        Err(Unavailability::UnreadableReply)
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

pub fn parse(provider: Provider, root: &Value, origin: Option<&str>) -> ProviderUsage {
    let mut windows = Vec::new();
    let mut plan = None;
    match provider {
        Provider::Gemini => {
            let mut least: std::collections::BTreeMap<&str, (&Value, f64)> = Default::default();
            for bucket in root["buckets"].as_array().into_iter().flatten() {
                if let (Some(model), Some(left)) = (
                    bucket["modelId"].as_str(),
                    num(&bucket["remainingFraction"]).filter(|v| *v <= 1.0),
                ) {
                    if least.get(model).is_none_or(|(_, old)| left < *old) {
                        least.insert(model, (bucket, left));
                    }
                }
            }
            for (model, (bucket, left)) in least {
                let mut window = UsageWindow::new(
                    &format!("gemini.{model}"),
                    Kind::Daily,
                    Some(model.into()),
                    1.0 - left,
                    86400,
                    stamp(&bucket["resetTime"]),
                );
                window.reports_length = false;
                windows.push(window);
            }
        }
        Provider::GrokBot => {
            if root["hasNonZeroIncludedLimit"].as_bool() == Some(true)
                && root["includedLimitZero"].as_bool() != Some(true)
                && root["usesPooledEnterpriseAllowance"].as_bool() != Some(true)
            {
                if let Some(percent) = num(&root["usagePercent"]) {
                    let mut window = UsageWindow::new(
                        "grokBot",
                        Kind::Weekly,
                        None,
                        (percent / 100.0).clamp(0.0, 1.0),
                        604800,
                        stamp(&root["nextResetTimestampUtc"]),
                    );
                    window.reports_length = false;
                    windows.push(window);
                }
            }
            plan = root["grokPlanLabel"].as_str().map(str::to_owned);
        }
        Provider::NousPortal => {
            if !root["error"].is_null() {
                return ProviderUsage::unavailable(
                    AccountKey::primary(provider),
                    Unavailability::ServerError,
                );
            }
            let sub = &root["subscription"];
            if [
                &sub["monthly_credits"],
                &sub["credits_remaining"],
                &root["paid_service_access"]["subscription_credits_remaining"],
                &root["paid_service_access"]["total_usable_credits"],
                &root["purchased_credits_remaining"],
            ]
            .iter()
            .any(|v| !v.is_null() && num(v).is_none())
            {
                return ProviderUsage::unavailable(
                    AccountKey::primary(provider),
                    Unavailability::UnreadableReply,
                );
            }
            if let (Some(total), Some(left)) = (
                num(&sub["monthly_credits"]).filter(|n| *n > 0.0),
                num(&sub["credits_remaining"]).or_else(|| {
                    num(&root["paid_service_access"]["subscription_credits_remaining"])
                }),
            ) {
                let mut window = UsageWindow::new(
                    "nousportal.monthly",
                    Kind::Monthly,
                    None,
                    ((total - left) / total).max(0.0),
                    2592000,
                    stamp(&sub["current_period_end"]),
                );
                window.reports_length = false;
                windows.push(window);
            }
            plan = sub["plan"].as_str().map(str::to_owned);
            let balance = num(&root["paid_service_access"]["total_usable_credits"])
                .or_else(|| num(&root["purchased_credits_remaining"]));
            let mut usage = reading(provider, windows, plan, origin.unwrap_or("endpoint"));
            if let Some(amount) = balance {
                usage.state = crate::model::State::Live;
                usage.observed_at = Some(crate::timeutil::now_ms());
                usage.credit_balance = Some(format!("{amount:.2} USD"));
                usage.credit_remaining = Some(CreditAmount {
                    amount,
                    currency: "USD".into(),
                });
            }
            return usage;
        }
        Provider::Devin => {
            plan = root["planName"].as_str().map(str::to_owned);
            for (id, kind, seconds, left, reset, hide) in [
                (
                    "daily",
                    Kind::Daily,
                    86400,
                    "dailyRemainingPercent",
                    "dailyResetAtUnix",
                    "hideDailyQuota",
                ),
                (
                    "weekly",
                    Kind::Weekly,
                    604800,
                    "weeklyRemainingPercent",
                    "weeklyResetAtUnix",
                    "hideWeeklyQuota",
                ),
            ] {
                if root[hide].as_bool() == Some(true) {
                    continue;
                }
                if let Some(left) = num(&root[left]).filter(|v| *v <= 100.0) {
                    windows.push(UsageWindow::new(
                        id,
                        kind,
                        None,
                        1.0 - left / 100.0,
                        seconds,
                        stamp(&root[reset]),
                    ));
                }
            }
            if windows.is_empty() {
                if let (Some(total), Some(left)) = (
                    num(&root["totalMessages"]).filter(|n| *n > 0.0),
                    num(&root["remainingMessages"]),
                ) {
                    let mut window = UsageWindow::new(
                        "messages",
                        Kind::Messages,
                        None,
                        (1.0 - left / total).max(0.0),
                        0,
                        None,
                    );
                    window.reports_length = false;
                    windows.push(window);
                }
            }
        }
        Provider::AlibabaTokenPlan => {
            let tree = find(root, "per5HourPercentage")
                .or_else(|| find(root, "per1WeekPercentage"))
                .unwrap_or(root);
            for (key, reset, id, kind, seconds) in [
                (
                    "per5HourPercentage",
                    "per5HourResetTime",
                    "fiveHour",
                    Kind::FiveHour,
                    18000,
                ),
                (
                    "per1WeekPercentage",
                    "per1WeekResetTime",
                    "weekly",
                    Kind::Weekly,
                    604800,
                ),
                (
                    "per1MonthPercentage",
                    "per1MonthResetTime",
                    "monthly",
                    Kind::Monthly,
                    2592000,
                ),
            ] {
                if let Some(ratio) = num(&tree[key]).filter(|v| *v <= 1.0) {
                    let mut window =
                        UsageWindow::new(id, kind, None, ratio, seconds, stamp(&tree[reset]));
                    window.reports_length = id != "monthly";
                    windows.push(window);
                }
            }
        }
        Provider::Kiro => {
            plan = root["data"]["planName"].as_str().map(str::to_owned);
            if root["success"].as_bool() == Some(true) {
                for item in root["data"]["usageBreakdowns"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if item["hasLimit"].as_bool() == Some(false) {
                        continue;
                    }
                    if let (Some(limit), Some(used)) = (
                        num(&item["limit"]).filter(|n| *n > 0.0),
                        num(&item["used"]).or_else(|| {
                            num(&item["percentage"])
                                .map(|p| p / 100.0 * num(&item["limit"]).unwrap_or(0.0))
                        }),
                    ) {
                        let id = item["resourceType"]
                            .as_str()
                            .or_else(|| item["displayName"].as_str())
                            .unwrap_or("usage");
                        let mut window = UsageWindow::new(
                            &id.to_lowercase(),
                            Kind::Monthly,
                            item["displayName"].as_str().map(str::to_owned),
                            (used / limit).clamp(0.0, 1.0),
                            2592000,
                            stamp(&root["data"]["billingCycleReset"]),
                        );
                        window.reports_length = false;
                        windows.push(window);
                    }
                }
            }
        }
        Provider::Volcengine => {
            for item in root["items"].as_array().into_iter().flatten() {
                if item["subscribed"].as_bool() == Some(false) {
                    continue;
                }
                let scope = match item["product"].as_str().unwrap_or("") {
                    "coding-plan" => "Coding Plan",
                    "agent-plan" => "Agent Plan",
                    "coding-plan-team" => "Coding Plan · Team",
                    "agent-plan-team" => "Agent Plan · Team",
                    _ => continue,
                };
                for row in item["periods"].as_array().into_iter().flatten() {
                    if let (Some(label), Some(percent)) =
                        (row["label"].as_str(), num(&row["percent"]))
                    {
                        if let Some(window) = super::volcengine_signer::window(
                            label,
                            percent,
                            stamp(&row["reset_at"]),
                            scope,
                        ) {
                            windows.push(window);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    reading(provider, windows, plan, origin.unwrap_or("localLogin"))
}
fn find<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    if root.get(key).is_some() {
        return Some(root);
    }
    match root {
        Value::Object(map) => map.values().find_map(|v| find(v, key)),
        Value::Array(rows) => rows.iter().find_map(|v| find(v, key)),
        _ => None,
    }
}
fn percent_text(text: &str) -> Option<f64> {
    let mut values = Vec::new();
    for (at, _) in text.match_indices('%') {
        let prefix = &text[..at];
        let digits: String = prefix
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if let Ok(value) = digits.parse::<f64>() {
            if (0.0..=100.0).contains(&value) {
                values.push(value);
            }
        }
    }
    values.sort_by(|a, b| a.total_cmp(b));
    values.dedup();
    (values.len() == 1).then(|| values[0])
}
fn ollama_cookie(header: &str) -> Option<String> {
    if header.len() > 32768 || header.bytes().any(|b| !(32..=126).contains(&b)) {
        return None;
    }
    let header = header.trim();
    let header = if header
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("cookie:"))
    {
        header[7..].trim()
    } else {
        header
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut pairs = Vec::new();
    for part in header.split(';') {
        let (name, value) = part.trim().split_once('=')?;
        let value = value.trim();
        let recognized = OLLAMA_SESSION.cookies[..4].iter().any(|base| {
            name == *base
                || name.strip_prefix(&format!("{base}.")).is_some_and(|tail| {
                    !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
                })
        });
        if recognized {
            if value.is_empty() || value.contains(' ') {
                return None;
            }
            if seen.insert(name) {
                pairs.push(format!("{name}={value}"));
            }
        }
    }
    (!pairs.is_empty()).then(|| pairs.join("; "))
}
fn jetbrains() -> Result<ProviderUsage, Unavailability> {
    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("JetBrains");
    let path = std::fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("options/AIAssistantQuotaManager2.xml"))
        .filter(|p| p.is_file())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
        .ok_or(Unavailability::LocalAppMissing)?;
    let xml = std::fs::read_to_string(&path).map_err(|_| Unavailability::UnreadableReply)?;
    let xml = xml
        .split("<component name=\"AIAssistantQuotaManager2\"")
        .nth(1)
        .and_then(|s| s.split("</component>").next())
        .ok_or(Unavailability::NoLimitsReported)?;
    let option = |key| {
        xml.split("<option")
            .find(|s| s.contains(&format!("name=\"{key}\"")))
            .and_then(|s| s.split_once("value=\""))
            .and_then(|(_, s)| s.split('"').next())
            .map(|s| {
                s.replace("&quot;", "\"")
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
            })
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    };
    let quota = option("quotaInfo").ok_or(Unavailability::NoLimitsReported)?;
    let refill = option("nextRefill").unwrap_or(Value::Null);
    let used = num(&quota["current"]).ok_or(Unavailability::NoLimitsReported)?;
    let total = num(&quota["maximum"])
        .filter(|n| *n > 0.0)
        .ok_or(Unavailability::NoLimitsReported)?;
    let mut window = UsageWindow::new(
        "jetbrains.quota",
        Kind::Credits,
        None,
        used / total,
        2592000,
        stamp(&refill["next"]),
    );
    window.reports_length = false;
    let mut usage = reading(Provider::JetBrainsAi, vec![window], None, "savedPlan");
    usage.state = crate::model::State::Stale;
    usage.is_cached = true;
    usage.observed_at = path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    Ok(usage)
}
enum Proto {
    Int(u64),
    Bytes(Vec<u8>),
}
fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push(value as u8 | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn proto(bytes: &[u8]) -> Result<Vec<(u64, Proto)>, Unavailability> {
    fn integer(bytes: &[u8], at: &mut usize) -> Option<u64> {
        let mut out = 0;
        for shift in (0..64).step_by(7) {
            let byte = *bytes.get(*at)?;
            *at += 1;
            out |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Some(out);
            }
        }
        None
    }
    let mut fields = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let key = integer(bytes, &mut at).ok_or(Unavailability::UnreadableReply)?;
        if key >> 3 == 0 {
            return Err(Unavailability::UnreadableReply);
        }
        let value = match key & 7 {
            0 => Proto::Int(integer(bytes, &mut at).ok_or(Unavailability::UnreadableReply)?),
            2 => {
                let len = integer(bytes, &mut at)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or(Unavailability::UnreadableReply)?;
                let end = at
                    .checked_add(len)
                    .filter(|e| *e <= bytes.len())
                    .ok_or(Unavailability::UnreadableReply)?;
                let v = bytes[at..end].to_vec();
                at = end;
                Proto::Bytes(v)
            }
            1 => {
                at += 8;
                continue;
            }
            5 => {
                at += 4;
                continue;
            }
            _ => return Err(Unavailability::UnreadableReply),
        };
        fields.push((key >> 3, value));
    }
    if at != bytes.len() {
        return Err(Unavailability::UnreadableReply);
    }
    Ok(fields)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unissued_grok_bot_and_unknown_gemini_quotas_are_not_zero() {
        assert!(parse(
            Provider::GrokBot,
            &json!({"usagePercent":0,"includedLimitZero":true}),
            None
        )
        .windows
        .is_empty());
        let reading = parse(
            Provider::Gemini,
            &json!({"buckets":[{"modelId":"model","remainingFraction":0.7},{"modelId":"model","remainingFraction":0.4},{"modelId":"missing"}]}),
            None,
        );
        assert_eq!(reading.windows.len(), 1);
        assert_eq!(reading.windows[0].used_fraction, 0.6);
        assert!(!reading.windows[0].reports_length);
    }
    #[test]
    fn billing_months_have_no_fabricated_length_and_free_balances_work() {
        let reading = parse(
            Provider::NousPortal,
            &json!({"purchased_credits_remaining":"2.50"}),
            None,
        );
        assert_eq!(reading.credit_remaining.unwrap().amount, 2.5);
        assert!(reading.windows.is_empty());
        let reading = parse(
            Provider::AlibabaTokenPlan,
            &json!({"data":{"per5HourPercentage":0.5,"per1MonthPercentage":50}}),
            None,
        );
        assert_eq!(reading.windows.len(), 1);
    }
    #[test]
    fn malformed_protobuf_and_ambiguous_html_are_rejected() {
        assert!(proto(&[10, 5, 1]).is_err());
        assert!(percent_text("20% and 30%").is_none());
        assert_eq!(percent_text("20% width=20%"), Some(20.0));
        assert_eq!(
            ollama_cookie("Cookie: analytics=private; wos-session.0=abc; wos-session.0=def"),
            Some("wos-session.0=abc".into())
        );
        assert!(ollama_cookie("wos-session=abc\r\nX: def").is_none());
    }
    #[test]
    fn quota_fixtures_keep_products_and_unknown_counters_separate() {
        let parse_fixture = |provider, text| {
            parse(
                provider,
                &serde_json::from_str::<Value>(text).unwrap(),
                None,
            )
        };
        let kiro = parse_fixture(
            Provider::Kiro,
            include_str!("../../tests/fixtures/upstream-kiro-pro-plus-usage.json"),
        );
        assert_eq!(kiro.windows.len(), 2);
        assert!((kiro.windows[0].used_fraction - 123.45 / 2000.0).abs() < 1e-10);
        let devin = parse_fixture(
            Provider::Devin,
            include_str!("../../tests/fixtures/upstream-devin-pro.json"),
        );
        assert_eq!(devin.windows.len(), 2);
        assert!((devin.windows[0].used_fraction - 0.02).abs() < 1e-10);
        let volc = parse_fixture(
            Provider::Volcengine,
            include_str!("../../tests/fixtures/upstream-volcengine-arkcli-usage.json"),
        );
        assert_eq!(volc.windows.len(), 4);
        assert!(volc
            .windows
            .iter()
            .any(|w| w.scope.as_deref() == Some("Agent Plan") && w.is_exhausted));
        let alibaba = parse_fixture(
            Provider::AlibabaTokenPlan,
            include_str!("../../tests/fixtures/upstream-alibaba-token-plan-cli-usage.json"),
        );
        assert_eq!(alibaba.windows.len(), 3);
        let nous = parse_fixture(
            Provider::NousPortal,
            include_str!("../../tests/fixtures/upstream-nous-portal-account.json"),
        );
        assert_eq!(nous.windows[0].used_fraction, 0.75);
        assert_eq!(nous.credit_remaining.unwrap().amount, 74.25);
        assert!(matches!(
            parse(
                Provider::NousPortal,
                &json!({"purchased_credits_remaining":-1}),
                None
            )
            .state,
            crate::model::State::Unavailable(Unavailability::UnreadableReply)
        ));
    }
    #[test]
    fn hermes_pool_uses_longest_lived_token_and_jwt_expiry() {
        let root = json!({"credential_pool":{"nous":[{"access_token":"old","expires_at":"2026-07-16T09:00:00Z"},{"access_token":"new","expires_at":"2026-07-16T12:00:00Z","portal_base_url":"https://evil.example"}]}});
        assert_eq!(nous_login(&root).unwrap().0, "new");
        assert_eq!(
            nous_login(&json!({"access_token":"x.eyJleHAiOjE3ODQxOTE2MDB9.y"}))
                .unwrap()
                .1,
            Some(1784191600000)
        );
    }
}
