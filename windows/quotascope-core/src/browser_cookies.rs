//! Reading one site's session cookie out of the browser the user signed in
//! with, so they don't have to copy it from developer tools by hand — the
//! Windows counterpart of `BrowserCookies.swift`.
//!
//! **Only the named hosts' cookies are ever looked at, and only the cookie
//! names the caller wants survive** — everything else is dropped before it
//! leaves this file. Nothing is stored here; what comes back goes straight
//! into the caller's own DPAPI-protected key store.
//!
//! **What it costs is different per browser, and it must be said honestly.**
//! Firefox keeps cookies in plain SQLite and costs nothing. Chromium encrypts
//! its values with a key in `Local State` that DPAPI protects for this user —
//! no prompt on Windows, unlike macOS's keychain — but Chrome 127 and later
//! move cookies behind App-Bound Encryption (`v20`), which this module
//! deliberately cannot open: a value that will not decrypt is dropped, never
//! guessed at.
//!
//! **The default browser is tried first regardless**, which is a decision
//! taken deliberately over trying the free ones first. The browser the user
//! opens links with is where they are actually signed in; the others may hold
//! a session that is months stale, and finding *that* is worse than finding
//! nothing. Each browser is independent: one failing tells the next nothing.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// The browsers this module knows how to read, in the order they are tried
/// when none of them is the default: the two mass-market Chromium ones first,
/// then the plain-SQLite one that costs nothing, then the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Edge,
    Firefox,
    Brave,
    Vivaldi,
}

pub const ALL: [Browser; 5] = [
    Browser::Chrome,
    Browser::Edge,
    Browser::Firefox,
    Browser::Brave,
    Browser::Vivaldi,
];

impl Browser {
    pub fn name(&self) -> &'static str {
        match self {
            Browser::Chrome => "Chrome",
            Browser::Edge => "Edge",
            Browser::Firefox => "Firefox",
            Browser::Brave => "Brave",
            Browser::Vivaldi => "Vivaldi",
        }
    }
}

/// One browser's answer: which browser it came from, and the `Cookie:` header
/// value — `name=value; name2=value2` — holding only the wanted names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub browser: Browser,
    pub header: String,
}

/// The directories the real machine's browser stores live under:
/// `%LOCALAPPDATA%` for the Chromium family, `%APPDATA%` for Firefox.
struct Bases {
    local: PathBuf,
    roaming: PathBuf,
}

fn bases() -> Bases {
    let local = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::model::home_path("AppData/Local"));
    let roaming = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::model::home_path("AppData/Roaming"));
    Bases { local, roaming }
}

/// Every cookie for `hosts` whose name the caller wants, browser by browser,
/// the default browser first. `cookies` holds the wanted names; a name ending
/// in `*` matches any cookie whose name starts with the rest (Mistral writes
/// one `ory_session_<token>` per session, and no fixed list can name them).
/// The first browser that yields any of the wanted names wins; its answers
/// are not merged with another browser's.
pub fn session(hosts: &[&str], cookies: &[&str]) -> Option<Found> {
    let bases = bases();
    session_in(&bases, hosts, cookies)
}
/// User-triggered import from Claude Desktop's Electron profile. App-Bound
/// encryption stays unsupported, just as for ordinary Chromium profiles.
pub fn claude_desktop_session() -> Option<String> {
    let root = bases().roaming.join("Claude");
    let key = local_state_key(&root)?;
    for path in [
        "Network/Cookies",
        "Cookies",
        "Default/Network/Cookies",
        "Default/Cookies",
    ] {
        let rows = chromium_cookies_at(&root.join(path), "claude.ai", &key);
        if rows
            .iter()
            .any(|(name, value)| name == "sessionKey" && !value.is_empty())
        {
            return Some(
                rows.into_iter()
                    .filter(|(name, _)| {
                        crate::providers::claude_session::COOKIES.contains(&name.as_str())
                    })
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
    }
    None
}

fn session_in(bases: &Bases, hosts: &[&str], cookies: &[&str]) -> Option<Found> {
    let mut browsers = present_in(bases);
    // The default browser moves to the front rather than being sorted:
    // "is it the default" is not an ordering.
    if let Some(preferred) = preferred_browser() {
        if let Some(at) = browsers.iter().position(|b| *b == preferred) {
            browsers.remove(at);
            browsers.insert(0, preferred);
        }
    }

    for host in hosts {
        for browser in &browsers {
            let pairs = match browser {
                Browser::Firefox => firefox(host, &bases.roaming),
                _ => {
                    let Some(root) = chromium_root(*browser, &bases.local) else {
                        continue;
                    };
                    let Some(key) = local_state_key(&root) else {
                        continue;
                    };
                    chromium(*browser, host, &bases.local, &key)
                }
            };
            let kept: Vec<(String, String)> = pairs
                .into_iter()
                .filter(|(name, value)| {
                    !value.is_empty() && cookies.iter().any(|want| name_matches(name, want))
                })
                .collect();
            if !kept.is_empty() {
                let header = kept
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Some(Found {
                    browser: *browser,
                    header,
                });
            }
        }
    }
    None
}

/// Which browsers are actually installed and have a cookie store — the
/// default one first, so the first thing tried is where the session is.
pub fn present() -> Vec<Browser> {
    present_in(&bases())
}

fn present_in(bases: &Bases) -> Vec<Browser> {
    ALL.into_iter()
        .filter(|browser| match browser {
            Browser::Firefox => !firefox_stores(&bases.roaming).is_empty(),
            _ => {
                let Some(root) = chromium_root(*browser, &bases.local) else {
                    return false;
                };
                !chromium_stores_in(&root).is_empty()
            }
        })
        .collect()
}

/// The browser this machine opens links with, read from the shell's own
/// association for `https:` — having Chrome on disk says nothing about
/// whether it is ever used.
fn preferred_browser() -> Option<Browser> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

        let path: Vec<u16> = "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice\0"
            .encode_utf16()
            .collect();
        let value: Vec<u16> = "ProgId\0".encode_utf16().collect();
        let mut buffer = [0u16; 128];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != windows::Win32::Foundation::ERROR_SUCCESS {
            return None;
        }
        let prog_id =
            String::from_utf16_lossy(&buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(0)]);
        if prog_id.contains("ChromeHTML") {
            Some(Browser::Chrome)
        } else if prog_id.contains("MSEdgeHTM") {
            Some(Browser::Edge)
        } else if prog_id.starts_with("FirefoxURL") || prog_id.starts_with("FirefoxHTML") {
            Some(Browser::Firefox)
        } else if prog_id.contains("Brave") {
            Some(Browser::Brave)
        } else if prog_id.contains("Vivaldi") {
            Some(Browser::Vivaldi)
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

// MARK: - Where each browser keeps them

fn chromium_root(browser: Browser, local: &Path) -> Option<PathBuf> {
    let root = match browser {
        Browser::Chrome => "Google/Chrome/User Data",
        Browser::Edge => "Microsoft/Edge/User Data",
        Browser::Brave => "BraveSoftware/Brave-Browser/User Data",
        Browser::Vivaldi => "Vivaldi/User Data",
        Browser::Firefox => return None,
    };
    Some(local.join(root))
}

fn chromium_stores_in(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut profiles: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name == "Default" || name.starts_with("Profile ")
        })
        .collect();
    profiles.sort();

    profiles
        .into_iter()
        .flat_map(|profile| [profile.join("Network/Cookies"), profile.join("Cookies")])
        .filter(|store| store.is_file())
        .collect()
}

fn firefox_stores(roaming: &Path) -> Vec<PathBuf> {
    let root = roaming.join("Mozilla/Firefox/Profiles");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut stores: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join("cookies.sqlite"))
        .filter(|store| store.is_file())
        .collect();
    stores.sort();
    stores
}

// MARK: - Firefox: plain SQLite, nothing to decrypt

fn firefox(host: &str, roaming: &Path) -> Vec<(String, String)> {
    for store in firefox_stores(roaming) {
        let rows = open_readonly(&store, |connection| {
            query_cookies(
                connection,
                "SELECT name, value FROM moz_cookies WHERE host = ?1 OR host = ?2 OR host LIKE ?3",
                host,
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
        });
        if !rows.is_empty() {
            return rows;
        }
    }
    Vec::new()
}

// MARK: - Chromium: SQLite, values encrypted with a key from Local State

fn chromium(browser: Browser, host: &str, local: &Path, key: &[u8]) -> Vec<(String, String)> {
    let Some(root) = chromium_root(browser, local) else {
        return Vec::new();
    };
    for store in chromium_stores_in(&root) {
        let rows = chromium_cookies_at(&store, host, key);
        if !rows.is_empty() {
            return rows;
        }
    }
    Vec::new()
}

/// One Chromium-format store, read for a single host. Internal so a test can
/// drive it against a store built on purpose with a known key.
///
/// The host match is the host, its dot-form and its subdomains, never a
/// suffix — asking for `claude.ai` must not return `notclaude.ai`, whose
/// value would then be joined into the header sent to the real one.
pub(crate) fn chromium_cookies_at(store: &Path, host: &str, key: &[u8]) -> Vec<(String, String)> {
    open_readonly(store, |connection| {
        query_cookies(
            connection,
            "SELECT name, encrypted_value FROM cookies WHERE host_key = ?1 OR host_key = ?2 OR host_key LIKE ?3",
            host,
            |row| {
                let name: String = row.get(0)?;
                let encrypted: Vec<u8> = row.get(1)?;
                let value = decrypt_value(&encrypted, key);
                Ok((name, value))
            },
        )
    })
    .into_iter()
    .filter_map(|(name, value)| value.map(|v| (name, v)))
    .collect()
}

/// The AES key a Chromium browser protects its cookie values with: a DPAPI
/// blob inside `Local State`, under `os_crypt.encrypted_key`. The blob
/// carries a five-byte `DPAPI` marker before the encrypted payload.
fn local_state_key(root: &Path) -> Option<Vec<u8>> {
    let data = std::fs::read(root.join("Local State")).ok()?;
    let root: Value = serde_json::from_slice(&data).ok()?;
    let encoded = root.get("os_crypt")?.get("encrypted_key")?.as_str()?;
    let blob = base64_decode(encoded)?;
    let payload = blob.strip_prefix(b"DPAPI")?;
    crate::secrets::dpapi_unprotect(payload)
}

/// One cookie value. `v10`/`v11` are AES-256-GCM: a twelve-byte nonce, then
/// the ciphertext with its sixteen-byte tag. `v20` is App-Bound Encryption —
/// the value exists but this module cannot and will not open it. Anything
/// else was never encrypted at all on older profiles.
fn decrypt_value(encrypted: &[u8], key: &[u8]) -> Option<String> {
    if encrypted.len() <= 3 {
        // Too short to carry a version prefix: plaintext, possibly empty.
        return String::from_utf8(encrypted.to_vec()).ok();
    }
    let prefix = &encrypted[..3];
    match prefix {
        b"v10" | b"v11" => {}
        b"v20" => return None,
        _ => return String::from_utf8(encrypted.to_vec()).ok(),
    }

    let body = &encrypted[3..];
    if body.len() < 12 + 16 {
        return None;
    }
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Nonce};
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = Nonce::from_slice(&body[..12]);
    let plain = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &body[12..],
                aad: &[],
            },
        )
        .ok()?;
    String::from_utf8(plain).ok()
}

/// A cookie name matches a wanted name exactly, or by prefix when the wanted
/// name ends in `*`.
fn name_matches(cookie_name: &str, wanted: &str) -> bool {
    match wanted.strip_suffix('*') {
        Some(prefix) => cookie_name.starts_with(prefix),
        None => cookie_name == wanted,
    }
}

/// Minimal standard base64 — worth its thirty lines so the crate grows no
/// dependency for one field of one JSON file.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for character in text.bytes() {
        if character == b'=' || character.is_ascii_whitespace() {
            continue;
        }
        let value = TABLE.iter().position(|t| *t == character)? as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// MARK: - SQLite, read-only, and out of the browser's way

/// Opens the store and runs one query. The browser may be running, and on
/// Windows it can hold the file in a way that blocks even a read-only open —
/// so a failed open falls back to a private copy, which the browser never
/// notices and nothing else ever touches.
fn open_readonly<T>(store: &Path, read: impl Fn(&rusqlite::Connection) -> T) -> T {
    let attempt = || -> rusqlite::Result<rusqlite::Connection> {
        rusqlite::Connection::open_with_flags(store, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    };
    match attempt() {
        Ok(connection) => read(&connection),
        Err(_) => {
            let copy = copied_aside(store);
            let Ok(connection) = rusqlite::Connection::open_with_flags(
                &copy,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                // `read` must return something; a store that cannot be opened
                // answers empty through the same path a missing table would.
                return read(&dead_connection());
            };
            read(&connection)
        }
    }
}

/// A connection to a guaranteed-empty in-memory database, for the one shape
/// where the closure must run and the store does not exist.
fn dead_connection() -> rusqlite::Connection {
    rusqlite::Connection::open_in_memory().expect("in-memory sqlite always opens")
}

fn copied_aside(store: &Path) -> PathBuf {
    let dir = std::env::temp_dir().join("quotascope-cookies");
    let _ = std::fs::create_dir_all(&dir);
    let copy = dir.join(format!(
        "{}-cookies.db",
        store
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("profile")
    ));
    let _ = std::fs::copy(store, &copy);
    // A write-ahead log that the browser is mid-way through belongs beside
    // the copy, or the copied database looks older than it is.
    for sidecar in ["-wal", "-shm"] {
        let from = store.with_file_name(format!(
            "{}{sidecar}",
            store.file_name().and_then(|n| n.to_str()).unwrap_or("")
        ));
        if from.is_file() {
            let _ = std::fs::copy(
                from,
                copy.with_file_name(format!(
                    "{}{sidecar}",
                    copy.file_name().and_then(|n| n.to_str()).unwrap_or("")
                )),
            );
        }
    }
    copy
}

fn query_cookies<T>(
    connection: &rusqlite::Connection,
    sql: &str,
    host: &str,
    read_row: impl Fn(&rusqlite::Row) -> rusqlite::Result<T>,
) -> Vec<T> {
    let Ok(mut statement) = connection.prepare(sql) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map(
        rusqlite::params![host, format!(".{host}"), format!("%.{host}")],
        read_row,
    ) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    fn bases_in(temp: &Path) -> Bases {
        Bases {
            local: temp.join("Local"),
            roaming: temp.join("Roaming"),
        }
    }

    /// The DPAPI layer is injected by the shell; tests inject a reversible
    /// stand-in, which is enough to drive the Local State path end to end.
    fn fake_dpapi() {
        fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
            data.iter().map(|b| b ^ 0x5A).collect::<Vec<_>>().into()
        }
        fn protect(data: &[u8]) -> Option<Vec<u8>> {
            data.iter().map(|b| b ^ 0x5A).collect::<Vec<_>>().into()
        }
        crate::secrets::install_dpapi(protect, unprotect);
    }

    fn encrypt_like_chromium(key: &[u8], value: &str) -> Vec<u8> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let nonce = Nonce::from_slice(&[7u8; 12]);
        let sealed = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: value.as_bytes(),
                    aad: &[],
                },
            )
            .expect("encrypts");
        let mut out = b"v10".to_vec();
        out.extend_from_slice(&[7u8; 12]);
        out.extend_from_slice(&sealed);
        out
    }

    fn make_chromium_store(root: &Path, profile: &str, rows: &[(&str, &str, &str)]) -> PathBuf {
        let store = root.join(profile).join("Network/Cookies");
        std::fs::create_dir_all(store.parent().expect("parent")).expect("mkdir");
        let connection = rusqlite::Connection::open(&store).expect("create");
        connection
            .execute_batch("CREATE TABLE cookies (name TEXT, encrypted_value BLOB, host_key TEXT);")
            .expect("create table");
        for (host, name, value) in rows {
            connection
                .execute(
                    "INSERT INTO cookies (host_key, name, encrypted_value) VALUES (?1, ?2, ?3)",
                    rusqlite::params![host, name, value.as_bytes()],
                )
                .expect("insert");
        }
        store
    }

    fn make_local_state(root: &Path, key: &[u8]) {
        // Every fixture must work independently of test order and scheduling.
        fake_dpapi();
        let protected: Vec<u8> = key.iter().map(|b| b ^ 0x5A).collect();
        let mut blob = b"DPAPI".to_vec();
        blob.extend_from_slice(&protected);
        let encoded = base64_encode(&blob);
        std::fs::create_dir_all(root).expect("mkdir");
        std::fs::write(
            root.join("Local State"),
            format!(r#"{{"os_crypt":{{"encrypted_key":"{encoded}"}}}}"#),
        )
        .expect("write");
    }

    #[test]
    fn chromium_values_decrypt_and_filter_by_name() {
        fake_dpapi();
        let temp = std::env::temp_dir().join(format!("qs-cookies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let bases = bases_in(&temp);
        let root = chromium_root(Browser::Chrome, &bases.local).expect("root");
        let key = [11u8; 32];
        make_local_state(&root, &key);
        make_chromium_store(
            &root,
            "Default",
            &[
                ("zed.dev", "zed.session", "session-token"),
                ("zed.dev", "other", "not-wanted"),
                ("notzed.dev", "zed.session", "impostor"),
            ],
        );

        let found = session_in(&bases, &["zed.dev"], &["zed.session"]).expect("found");
        assert_eq!(found.browser, Browser::Chrome);
        assert_eq!(found.header, "zed.session=session-token");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn subdomains_match_but_unrelated_suffixes_do_not() {
        let temp = std::env::temp_dir().join(format!("qs-cookies-sub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let bases = bases_in(&temp);
        let root = chromium_root(Browser::Chrome, &bases.local).expect("root");
        let key = [3u8; 32];
        make_local_state(&root, &key);
        make_chromium_store(
            &root,
            "Default",
            &[
                ("api.zed.dev", "zed.session", "sub"),
                ("notzed.dev", "zed.session", "impostor"),
            ],
        );
        let found = session_in(&bases, &["zed.dev"], &["zed.session"]).expect("found");
        assert_eq!(found.header, "zed.session=sub");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn a_wildcard_name_takes_every_session_cookie_of_that_shape() {
        let temp = std::env::temp_dir().join(format!("qs-cookies-wild-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let bases = bases_in(&temp);
        let root = chromium_root(Browser::Edge, &bases.local).expect("root");
        let key = [5u8; 32];
        make_local_state(&root, &key);
        make_chromium_store(
            &root,
            "Default",
            &[
                ("auth.mistral.ai", "ory_session_ABC", "one"),
                ("auth.mistral.ai", "ory_session_DEF", "two"),
                ("auth.mistral.ai", "csrf", "other"),
            ],
        );
        let found = session_in(&bases, &["auth.mistral.ai"], &["ory_session_*"]).expect("found");
        assert_eq!(found.header, "ory_session_ABC=one; ory_session_DEF=two");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn app_bound_values_are_dropped_not_guessed() {
        let key = [9u8; 32];
        assert_eq!(decrypt_value(b"v20-not-really-encrypted", &key), None);
    }

    #[test]
    fn gcm_values_round_trip_and_rubbish_is_rejected() {
        let key = [4u8; 32];
        let sealed = encrypt_like_chromium(&key, "the-value");
        assert_eq!(decrypt_value(&sealed, &key).as_deref(), Some("the-value"));
        // A flipped bit anywhere in the ciphertext fails the tag.
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert_eq!(decrypt_value(&tampered, &key), None);
        // Older profiles store some values in the clear.
        assert_eq!(
            decrypt_value(b"plain-value", &key).as_deref(),
            Some("plain-value")
        );
    }

    #[test]
    fn firefox_cookies_read_in_the_clear() {
        let temp = std::env::temp_dir().join(format!("qs-cookies-ff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let bases = bases_in(&temp);
        let profile = bases.roaming.join("Mozilla/Firefox/Profiles/abc.default");
        std::fs::create_dir_all(&profile).expect("mkdir");
        let connection =
            rusqlite::Connection::open(profile.join("cookies.sqlite")).expect("create");
        connection
            .execute_batch("CREATE TABLE moz_cookies (name TEXT, value TEXT, host TEXT);")
            .expect("create table");
        for (host, name, value) in [
            ("ollama.com", "session", "ff-token"),
            ("notollama.com", "session", "impostor"),
        ] {
            connection
                .execute(
                    "INSERT INTO moz_cookies (host, name, value) VALUES (?1, ?2, ?3)",
                    rusqlite::params![host, name, value],
                )
                .expect("insert");
        }

        let found = session_in(&bases, &["ollama.com"], &["session"]).expect("found");
        assert_eq!(found.browser, Browser::Firefox);
        assert_eq!(found.header, "session=ff-token");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn base64_decodes_the_local_state_shape() {
        let blob = b"DPAPI-hello";
        let encoded = base64_encode(blob);
        assert_eq!(base64_decode(&encoded).as_deref(), Some(blob.as_slice()));
        assert_eq!(base64_decode("not*valid"), None);
    }

    /// The encoder the fixtures need to hand `Local State` a plausible blob.
    fn base64_encode(data: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let bytes = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | bytes[2] as u32;
            out.push(TABLE[(n >> 18) as usize & 63] as char);
            out.push(TABLE[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                TABLE[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }
}
