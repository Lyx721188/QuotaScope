//! Additional upstream catalogue sources. Native formats are dispatched
//! explicitly. Every catalogue entry also accepts a documented local JSONL
//! export, rather than guessing counters from arbitrary objects.
use crate::ledger::{TokenTally, UsageLedger};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::{cell::Cell, rc::Rc};

const MAX_IMPORT_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IMPORT_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IMPORT_RECORDS: usize = 100_000;
const MAX_IMPORT_RECORD_BYTES: usize = 8 * 1024 * 1024;

struct ImportInput<R> {
    inner: R,
    bytes: Rc<Cell<u64>>,
    maximum: u64,
}
impl<R: Read> Read for ImportInput<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let used = self.bytes.get();
        if used > self.maximum {
            return Err(std::io::Error::other("usage import byte budget exhausted"));
        }
        let remaining = self.maximum - used;
        if remaining == 0 {
            let count = self.inner.read(&mut [0u8; 1])?;
            self.bytes.set(used + count as u64);
            return if count == 0 {
                Ok(0)
            } else {
                Err(std::io::Error::other("usage import byte budget exhausted"))
            };
        }
        let maximum = remaining.min(buffer.len() as u64) as usize;
        let count = self.inner.read(&mut buffer[..maximum])?;
        self.bytes.set(used + count as u64);
        Ok(count)
    }
}

#[derive(Default)]
struct ImportBudget {
    records: usize,
    estimated_bytes: usize,
    exhausted: bool,
}
impl ImportBudget {
    fn allows(&mut self, row: &Value) -> bool {
        let identity = row["id"].as_str().map_or(0, str::len);
        let model = row["model"].as_str().map_or(0, str::len);
        let charge = identity
            .saturating_add(model.saturating_mul(2))
            .saturating_add(224);
        self.records += 1;
        self.estimated_bytes = self.estimated_bytes.saturating_add(charge);
        self.exhausted |= self.records > MAX_IMPORT_RECORDS
            || self.estimated_bytes > MAX_IMPORT_RECORD_BYTES
            || !crate::scan::checkpoint();
        !self.exhausted
    }
}

struct ImportVisitor<'a> {
    source: &'a str,
    file: &'a Path,
    reader: &'a mut Reader,
    budget: &'a mut ImportBudget,
}
impl ImportVisitor<'_> {
    fn consume<E: serde::de::Error>(&mut self, row: Value) -> Result<(), E> {
        if !self.budget.allows(&row) {
            return Err(E::custom("usage import record budget exhausted"));
        }
        parse(
            self.source,
            &row,
            true,
            self.file,
            self.reader,
            &mut (0, None),
        );
        Ok(())
    }
}
impl<'de> serde::de::Visitor<'de> for ImportVisitor<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a usage object or an array of usage objects")
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(mut self, mut sequence: A) -> Result<(), A::Error> {
        loop {
            if !crate::scan::checkpoint() {
                return Err(serde::de::Error::custom("scan cancelled"));
            }
            let Some(row) = sequence.next_element::<Value>()? else {
                return Ok(());
            };
            self.consume(row)?;
        }
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(mut self, map: A) -> Result<(), A::Error> {
        let row =
            serde::Deserialize::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
        self.consume(row)
    }
}

fn read_import_json(
    source: &str,
    file: &Path,
    input: impl Read + 'static,
    reader: &mut Reader,
    budget: &mut ImportBudget,
) {
    let buffered = std::io::BufReader::with_capacity(
        64 * 1024,
        crate::zstd_stream::plain_limit(input, MAX_IMPORT_FILE_BYTES),
    );
    let mut decoder = serde_json::Deserializer::from_reader(buffered);
    let result = serde::Deserializer::deserialize_any(
        &mut decoder,
        ImportVisitor {
            source,
            file,
            reader,
            budget,
        },
    )
    .and_then(|_| decoder.end());
    if result.is_err() {
        reader.partial = true;
    }
    crate::scan::file_read();
}

pub const CATALOG: &[(&str, &str, &str)] = &[
    ("grok", "Grok Build", ".grok/sessions"),
    ("kimi", "Kimi CLI", ".kimi/sessions"),
    ("devin-cli", "Devin", ".local/share/devin/cli/sessions.db"),
    ("roocode", "Roo Code", ""),
    ("kilocode", "Kilo Code", ""),
    ("cline", "Cline", ""),
    ("codebuddy", "CodeBuddy", ".codebuddy"),
    ("workbuddy", "WorkBuddy", ".workbuddy"),
    ("cherrystudio", "Cherry Studio", ""),
    ("commandcode", "Command Code", ".commandcode"),
    ("opencodereview", "OpenCodeReview", ".opencodereview"),
    ("zcode", "ZCode", ".zcode/cli/db/db.sqlite"),
    ("hermes", "Hermes", ".hermes"),
    ("goose", "Goose", ".local/share/goose/sessions"),
    ("zed", "Zed", ""),
    ("kiro", "Kiro", ".kiro"),
    ("crush", "Crush", ".local/share/crush"),
    ("unsloth", "Unsloth", ".unsloth"),
    (
        "antigravity-cli",
        "Antigravity CLI",
        ".gemini/antigravity/conversations",
    ),
    ("micode", "MiMo Code", ".micode"),
    ("devin-desktop", "Devin Desktop", ""),
    ("freebuff", "Freebuff", ".freebuff"),
    ("dsh", "DeepSeek Harness", ".dsh/sessions"),
    ("cursor", "Cursor (export)", ".config/tokscale/cursor-cache"),
    (
        "antigravity",
        "Antigravity (export)",
        ".config/tokscale/antigravity-cache/sessions",
    ),
    (
        "trae",
        "Trae (export)",
        ".config/tokscale/trae-cache/sessions",
    ),
    ("warp", "Warp (export)", ".config/tokscale/warp-cache"),
    ("hindsight", "Hindsight (export)", ".hindsight/usage"),
    (
        "mcode",
        "MiniMax Code (export)",
        ".config/tokscale/headless/mcode",
    ),
    ("copilot", "GitHub Copilot", ".copilot/session-state"),
];

pub fn native_supported(id: &str) -> bool {
    matches!(
        id,
        "grok"
            | "kimi"
            | "devin-cli"
            | "roocode"
            | "kilocode"
            | "cline"
            | "dsh"
            | "zcode"
            | "antigravity"
            | "hindsight"
            | "cursor"
    )
}

pub fn roots(id: &str, relative: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if !relative.is_empty() {
        paths.push(crate::home_dir().join(relative));
    }
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::home_dir().join("AppData/Roaming"));
    if let Some(extension) = match id {
        "roocode" => Some("rooveterinaryinc.roo-cline"),
        "kilocode" => Some("kilocode.kilo-code"),
        "cline" => Some("saoudrizwan.claude-dev"),
        _ => None,
    } {
        for editor in ["Code", "Code - Insiders", "VSCodium"] {
            paths.push(
                roaming
                    .join(editor)
                    .join("User/globalStorage")
                    .join(extension)
                    .join("tasks"),
            );
        }
        for server in [".vscode-server", ".vscode-server-insiders"] {
            paths.push(
                crate::home_dir()
                    .join(server)
                    .join("data/User/globalStorage")
                    .join(extension)
                    .join("tasks"),
            );
        }
    }
    match id {
        "cherrystudio" => paths.push(roaming.join("CherryStudio")),
        "zed" => paths.push(
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join("Zed"),
        ),
        "devin-desktop" => paths.push(roaming.join("Devin/User/acp-events")),
        "devin-cli" => paths.push(roaming.join("devin/cli/sessions.db")),
        _ => {}
    }
    paths.push(crate::data_dir().join("UsageImports").join(id));
    paths
}

#[derive(Default)]
struct Reader {
    buckets: BTreeMap<String, BTreeMap<String, TokenTally>>,
    ids: HashMap<String, (String, String, TokenTally)>,
    partial: bool,
}
impl Reader {
    fn forget(&mut self, id: &str) {
        if let Some((slot, model, old)) = self.ids.remove(id) {
            if let Some(models) = self.buckets.get_mut(&slot) {
                if let Some(previous) = models.get_mut(&model) {
                    previous.input -= old.input;
                    previous.output -= old.output;
                    previous.cache_write -= old.cache_write;
                    previous.cache_read -= old.cache_read;
                    if previous.total() == 0 {
                        models.remove(&model);
                    }
                }
                if models.is_empty() {
                    self.buckets.remove(&slot);
                }
            }
        }
    }
    fn add(&mut self, at: i64, model: &str, tally: TokenTally, id: Option<String>) {
        if tally.total() <= 0 || chrono::DateTime::from_timestamp_millis(at).is_none() || at <= 0 {
            return;
        }
        let key = crate::ledger::slot_key_from_ms(at);
        if let Some(id) = id {
            self.forget(&id);
            self.ids.insert(id, (key.clone(), model.to_string(), tally));
        }
        *self
            .buckets
            .entry(key)
            .or_default()
            .entry(model.to_string())
            .or_default() += tally;
    }
}

pub fn read(id: &str, paths: &[PathBuf]) -> UsageLedger {
    let reader = read_source(id, paths);
    price_source(reader, &crate::model_prices::prices())
}

/// Offline verification and callers that already own a model-price snapshot.
pub fn read_with_prices(
    id: &str,
    paths: &[PathBuf],
    prices: &BTreeMap<String, crate::model_prices::ModelPrice>,
) -> UsageLedger {
    price_source(read_source(id, paths), prices)
}
fn price_source(
    reader: Reader,
    prices: &BTreeMap<String, crate::model_prices::ModelPrice>,
) -> UsageLedger {
    let mut ledger = crate::ledger::priced(&reader.buckets, prices, None);
    ledger.has_partial_records = reader.partial;
    ledger
}

fn read_source(id: &str, paths: &[PathBuf]) -> Reader {
    let mut reader = Reader::default();
    let mut files = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut import_files = Vec::new();
    let mut import_seen = HashSet::new();
    let mut import_listed = 0;
    let mut import_roots = 0;
    if id == "zcode" && paths.len() > 16 {
        reader.partial = true;
    }
    for path in paths
        .iter()
        .take(if id == "zcode" { 16 } else { paths.len() })
    {
        if path.components().any(|p| p.as_os_str() == "UsageImports") {
            import_roots += 1;
            if import_roots > 16 {
                reader.partial = true;
                continue;
            }
            collect_imports(
                path,
                0,
                &mut import_files,
                &mut import_seen,
                &mut import_listed,
                &mut reader.partial,
            );
            continue;
        }
        if id == "zcode" {
            let candidates = if path.is_dir() {
                vec![path.join("db.sqlite"), path.join("cli/db/db.sqlite")]
            } else {
                vec![path.clone()]
            };
            for candidate in candidates {
                if candidate.file_name().is_none_or(|name| name != "db.sqlite") {
                    continue;
                }
                if let Ok(canonical) = candidate.canonicalize() {
                    if seen_paths.insert(canonical) {
                        files.push(candidate);
                    }
                } else if candidate.try_exists().unwrap_or(true) {
                    reader.partial = true;
                }
            }
            continue;
        }
        if !native_supported(id) {
            continue;
        }
        collect(path, &mut files, &mut seen_paths);
    }
    files.extend(import_files);
    // Snapshot replacement must not depend on filesystem enumeration order.
    files.sort();
    files.dedup();
    let mut zcode_database = false;
    let mut import_bytes = 0_u64;
    let mut import_budget = ImportBudget::default();
    let import_read_bytes = Rc::new(Cell::new(0));
    for file in files {
        if !crate::scan::checkpoint() {
            reader.partial = true;
            break;
        }
        let import = file.components().any(|p| p.as_os_str() == "UsageImports");
        if import && import_budget.exhausted {
            continue;
        }
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if id == "zcode" && !import {
            // One official database owns all ZCode attempts. Never add a second
            // copy of that database as if it represented additional requests.
            if zcode_database {
                reader.partial = true;
                continue;
            }
            zcode_database = true;
            let report = crate::zcode_spend::read_cached(&file);
            for (slot, models) in &report.buckets {
                for (model, tally) in models {
                    if !crate::scan::checkpoint() {
                        reader.partial = true;
                        break;
                    }
                    *reader
                        .buckets
                        .entry(slot.clone())
                        .or_default()
                        .entry(model.clone())
                        .or_default() += *tally;
                }
            }
            reader.partial |= report.partial;
            continue;
        }
        if id == "devin-cli" && name == "sessions.db" && !import {
            devin(&file, &mut reader);
            continue;
        }
        let accepted = if import {
            matches!(
                file.extension().and_then(|v| v.to_str()),
                Some("jsonl" | "json")
            )
        } else {
            match id {
                "grok" => name == "updates.jsonl",
                "kimi" => name == "wire.jsonl",
                "roocode" | "kilocode" | "cline" => name == "ui_messages.json",
                "dsh" => {
                    name.starts_with("session")
                        && (name.ends_with(".jsonl") || name.ends_with(".zstd"))
                }
                "antigravity" | "hindsight" | "cursor" => {
                    file.extension().is_some_and(|s| s == "jsonl")
                }
                _ => false,
            }
        };
        if !accepted {
            continue;
        }
        let dsh = id == "dsh" && !import;
        let stamp = file
            .metadata()
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?)));
        let maximum = if import {
            MAX_IMPORT_FILE_BYTES
        } else if dsh {
            crate::zstd_stream::MAX_BYTES
        } else {
            128 * 1024 * 1024
        };
        if stamp.is_some_and(|(length, _)| length > maximum) {
            reader.partial = true;
            continue;
        }
        if import {
            import_bytes = import_bytes.saturating_add(stamp.map(|s| s.0).unwrap_or(maximum));
            if import_bytes > MAX_IMPORT_TOTAL_BYTES {
                reader.partial = true;
                continue;
            }
        }
        let Ok(mut input) = std::fs::File::open(&file) else {
            reader.partial = true;
            continue;
        };
        let mut magic = [0_u8; 4];
        let _ = input.read_exact(&mut magic);
        let compressed = magic == [0x28, 0xb5, 0x2f, 0xfd];
        if compressed && !dsh {
            reader.partial = true;
            continue;
        }
        if file.extension().is_some_and(|s| s == "json") {
            if import {
                if input.seek(SeekFrom::Start(0)).is_err() {
                    reader.partial = true;
                } else {
                    let input = ImportInput {
                        inner: input,
                        bytes: import_read_bytes.clone(),
                        maximum: MAX_IMPORT_TOTAL_BYTES,
                    };
                    read_import_json(id, &file, input, &mut reader, &mut import_budget);
                }
            } else if let Ok(value) = std::fs::read(&file).and_then(|bytes| {
                serde_json::from_slice::<Value>(&bytes).map_err(std::io::Error::other)
            }) {
                if let Some(rows) = value.as_array() {
                    for row in rows {
                        parse(id, row, import, &file, &mut reader, &mut (0, None));
                    }
                }
                crate::scan::file_read();
            } else {
                reader.partial = true;
            }
        } else {
            if input.seek(SeekFrom::Start(0)).is_err() {
                reader.partial = true;
                continue;
            }
            let input: Box<dyn Read> = if compressed {
                match crate::zstd_stream::decode(input) {
                    Ok(decoded) => decoded,
                    Err(_) => {
                        reader.partial = true;
                        continue;
                    }
                }
            } else if import {
                crate::zstd_stream::plain_limit(
                    ImportInput {
                        inner: input,
                        bytes: import_read_bytes.clone(),
                        maximum: MAX_IMPORT_TOTAL_BYTES,
                    },
                    maximum,
                )
            } else if dsh {
                crate::zstd_stream::plain(input)
            } else {
                Box::new(input)
            };
            let hash = std::collections::hash_map::DefaultHasher::new();
            let mut lines = if dsh || import {
                crate::scan::LineReader::with_line_limit(input, crate::zstd_stream::MAX_LINE_BYTES)
            } else {
                crate::scan::LineReader::new(input, hash, 0)
            };
            let mut context = (0, None);
            for line in lines.by_ref() {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if import && !import_budget.allows(&value) {
                        reader.partial = true;
                        break;
                    }
                    parse(id, &value, import, &file, &mut reader, &mut context);
                } else {
                    reader.partial = true;
                }
            }
            if lines.finish().is_err() {
                reader.partial = true;
            }
        }
        if (dsh || import)
            && (stamp.is_none()
                || file
                    .metadata()
                    .ok()
                    .and_then(|m| Some((m.len(), m.modified().ok()?)))
                    != stamp)
        {
            reader.partial = true;
        }
    }
    if !native_supported(id)
        && paths
            .iter()
            .any(|p| p.exists() && !p.components().any(|c| c.as_os_str() == "UsageImports"))
    {
        reader.partial = true;
    }
    reader
}

fn collect_imports(
    path: &Path,
    depth: usize,
    out: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    listed: &mut usize,
    partial: &mut bool,
) {
    if !crate::scan::checkpoint() || depth > 8 || *listed >= 10_000 || out.len() >= 1024 {
        *partial = true;
        return;
    }
    *listed += 1;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            *partial = true;
            return;
        }
    };
    if meta.file_type().is_symlink() {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return;
        }
    }
    if !seen.insert(path.to_path_buf()) {
        return;
    }
    if meta.is_file() {
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("json" | "jsonl")
        ) {
            out.push(path.to_path_buf());
        }
    } else if meta.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            *partial = true;
            return;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                *partial = true;
                continue;
            };
            collect_imports(&entry.path(), depth + 1, out, seen, listed, partial);
            if *listed >= 10_000 || out.len() >= 1024 || !crate::scan::checkpoint() {
                *partial = true;
                break;
            }
        }
    }
}

fn collect(path: &Path, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    if !crate::scan::checkpoint() || !seen.insert(path.to_path_buf()) {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_file() {
        out.push(path.to_path_buf());
    } else if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_name() != "binaries" {
                collect(&entry.path(), out, seen);
            }
        }
    }
}

fn count(value: &Value) -> i64 {
    crate::http::number(value)
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1e12)
        .map(|n| n as i64)
        .unwrap_or(0)
}
fn timestamp(value: &Value, numeric_ms: bool) -> Option<i64> {
    value
        .as_str()
        .and_then(crate::timeutil::parse_iso8601_ms)
        .or_else(|| {
            crate::http::number(value)
                .filter(|n| n.is_finite() && *n > 0.0)
                .map(|n| {
                    if numeric_ms {
                        n as i64
                    } else {
                        (n * 1000.0) as i64
                    }
                })
        })
}
fn anthropic(usage: &Value) -> TokenTally {
    TokenTally {
        input: count(&usage["input_tokens"]),
        output: count(&usage["output_tokens"]),
        cache_write: count(&usage["cache_creation_input_tokens"]),
        cache_read: count(&usage["cache_read_input_tokens"]),
    }
}
fn camel(usage: &Value) -> TokenTally {
    TokenTally {
        input: count(&usage["inputTokens"]),
        output: count(&usage["outputTokens"]),
        cache_write: count(&usage["cacheWriteTokens"]),
        cache_read: count(&usage["cacheReadTokens"]),
    }
}

fn dsh_tally(usage: &Value) -> Option<TokenTally> {
    let usage = usage.as_object()?;
    let required = |key| {
        usage
            .get(key)?
            .as_i64()
            .filter(|n| (0..=1_000_000_000_000).contains(n))
    };
    let optional = |key| match usage.get(key) {
        None => Some(0),
        Some(value) => value
            .as_i64()
            .filter(|n| (0..=1_000_000_000_000).contains(n)),
    };
    Some(TokenTally {
        input: required("inputTokens")?,
        output: required("outputTokens")?,
        cache_write: optional("cacheWriteTokens")?,
        cache_read: optional("cacheReadTokens")?,
    })
}

fn import_tally(usage: &Value) -> Option<TokenTally> {
    let tally = dsh_tally(usage)?;
    if let Some(reasoning) = usage.get("reasoningTokens") {
        if !(0..=tally.output).contains(&reasoning.as_i64()?) {
            return None;
        }
    }
    if let Some(total) = usage.get("totalTokens") {
        if total.as_i64()? != tally.total() {
            return None;
        }
    }
    Some(tally)
}

fn import_time(value: &Value) -> Option<i64> {
    let at = value.as_i64().or_else(|| {
        let text = value.as_str().filter(|text| text.len() <= 128)?;
        // A date without a time/zone is not a timestamp for an hourly ledger.
        chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|at| at.timestamp_millis())
    })?;
    if at <= 0
        || chrono::DateTime::from_timestamp_millis(at)
            .and_then(|at| at.checked_add_signed(chrono::Duration::days(1)))
            .is_none()
    {
        return None;
    }
    Some(at)
}

fn parse(
    id: &str,
    row: &Value,
    imported: bool,
    file: &Path,
    reader: &mut Reader,
    context: &mut (i64, Option<String>),
) {
    if imported {
        if row["schema"].as_str() != Some("quotascope.usage.v1")
            || row["source"].as_str() != Some(id)
        {
            reader.partial = true;
            return;
        }
        if let (Some(at), Some(record), Some(tally)) = (
            import_time(&row["timestamp"]),
            row["id"]
                .as_str()
                .filter(|id| !id.trim().is_empty() && id.len() <= 1024),
            import_tally(&row["usage"]),
        ) {
            let model = match row["model"]
                .as_str()
                .filter(|m| !m.trim().is_empty() && m.len() <= 256)
            {
                Some(model) => model.trim(),
                None => {
                    reader.partial = true;
                    "unknown"
                }
            };
            let identity = format!("export:{record}");
            // Explicitly reported zero is a replacement, not missing usage.
            if tally.total() == 0 {
                reader.forget(&identity);
            } else {
                reader.add(at, model, tally, Some(identity));
            }
        } else {
            reader.partial = true;
        }
        return;
    }
    let file_id = file
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    match id {
        "grok" => {
            let update = &row["params"]["update"];
            if update["sessionUpdate"].as_str() != Some("turn_completed") {
                return;
            }
            let Some(at) = timestamp(&row["timestamp"], false) else {
                return;
            };
            let usage = &update["usage"];
            let records = usage["modelUsage"]
                .as_object()
                .cloned()
                .unwrap_or_else(|| serde_json::Map::from_iter([("grok".into(), usage.clone())]));
            for (model, usage) in records {
                reader.add(
                    at,
                    &model,
                    TokenTally {
                        input: count(&usage["inputTokens"]),
                        output: count(&usage["outputTokens"]) + count(&usage["reasoningTokens"]),
                        cache_write: count(&usage["cacheCreationTokens"]),
                        cache_read: count(&usage["cachedReadTokens"]),
                    },
                    Some(format!("{file_id}:{at}:{model}")),
                );
            }
        }
        "kimi" => {
            let usage = &row["message"]["payload"]["token_usage"];
            let Some(at) = timestamp(&row["timestamp"], count(&row["timestamp"]) > 10_000_000_000)
            else {
                return;
            };
            reader.add(
                at,
                "kimi (unnamed)",
                TokenTally {
                    input: count(&usage["input_other"]),
                    output: count(&usage["output"]),
                    cache_write: count(&usage["input_cache_creation"]),
                    cache_read: count(&usage["input_cache_read"]),
                },
                Some(format!("{file_id}:{at}")),
            );
        }
        "roocode" | "kilocode" | "cline" => {
            if row["type"].as_str() != Some("say") || row["say"].as_str() != Some("api_req_started")
            {
                return;
            }
            if let (Some(at), Some(text), Some(model)) = (
                timestamp(&row["ts"], true),
                row["text"].as_str(),
                row["modelInfo"]["modelId"].as_str(),
            ) {
                if let Ok(usage) = serde_json::from_str::<Value>(text) {
                    reader.add(
                        at,
                        model,
                        TokenTally {
                            input: count(&usage["tokensIn"]),
                            output: count(&usage["tokensOut"]),
                            cache_write: count(&usage["cacheWrites"]),
                            cache_read: count(&usage["cacheReads"]),
                        },
                        Some(format!("{file_id}:{at}")),
                    );
                }
            }
        }
        "dsh" => {
            let kind = row["type"].as_str().unwrap_or("");
            if kind == "session" {
                context.0 = count(&row["seedLength"]);
            }
            if kind == "request/header" {
                if let Some(model) = row["data"]["header"]["config"]["model"].as_str() {
                    context.1 = Some(model.into());
                }
            }
            if kind != "assistant/message" || count(&row["seq"]) < context.0 {
                return;
            }
            let data = &row["data"];
            let source = &data["message"]["source"];
            let model = source["replayState"]["response"]["responseModel"]
                .as_str()
                .or_else(|| source["model"].as_str())
                .or(context.1.as_deref());
            if let (Some(at), Some(model)) = (timestamp(&row["time"], true), model) {
                let Some(tally) = dsh_tally(&data["usage"]) else {
                    reader.partial = true;
                    return;
                };
                let identity = data["message"]["id"]
                    .as_str()
                    .or_else(|| data["compactionId"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("seq:{}", count(&row["seq"])));
                reader.add(
                    at,
                    model,
                    tally,
                    Some(format!(
                        "dsh:{identity}:{at}:{model}:{}",
                        serde_json::to_string(&tally).unwrap_or_default()
                    )),
                );
            } else {
                reader.partial = true;
            }
        }
        "antigravity" => {
            if let (Some(at), Some(model)) =
                (timestamp(&row["timestamp"], true), row["modelId"].as_str())
            {
                let tally = TokenTally {
                    input: count(&row["input"]),
                    output: count(&row["output"]),
                    cache_write: count(&row["cacheWrite"]),
                    cache_read: count(&row["cacheRead"]),
                };
                reader.add(at, model, tally, Some(format!("{model}:{at}")));
            }
        }
        "cursor" => {
            if let (Some(at), Some(model)) =
                (timestamp(&row["timestamp"], true), row["model"].as_str())
            {
                reader.add(
                    at,
                    model,
                    camel(&row["usage"]),
                    Some(format!("{model}:{at}")),
                );
            }
        }
        "hindsight" => {
            if let (Some(at), Some(model)) =
                (timestamp(&row["started_at"], false), row["model"].as_str())
            {
                let read = count(&row["cached_tokens"]);
                reader.add(
                    at,
                    model,
                    TokenTally {
                        input: (count(&row["input_tokens"]) - read).max(0),
                        output: count(&row["output_tokens"]),
                        cache_read: read,
                        cache_write: 0,
                    },
                    row["id"].as_str().map(|id| format!("hindsight:{id}")),
                );
            }
        }
        _ => {}
    }
}

fn devin(path: &Path, reader: &mut Reader) {
    let Ok(db) =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        reader.partial = true;
        return;
    };
    let Ok(mut q) = db.prepare("SELECT session_id, chat_message, created_at FROM message_nodes")
    else {
        reader.partial = true;
        return;
    };
    let Ok(rows) = q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    }) else {
        reader.partial = true;
        return;
    };
    for row in rows {
        if !crate::scan::checkpoint() {
            reader.partial = true;
            break;
        }
        let Ok((session, message, time)) = row else {
            reader.partial = true;
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&message) else {
            reader.partial = true;
            continue;
        };
        let metadata = &value["metadata"];
        let usage = &metadata["metrics"];
        let mut tally = anthropic(usage);
        tally.cache_read = count(&usage["cache_read_tokens"]);
        tally.cache_write = count(&usage["cache_creation_tokens"]);
        if let Some(at) = timestamp(&Value::from(time), time > 10_000_000_000) {
            reader.add(
                at,
                metadata["generation_model"].as_str().unwrap_or("devin"),
                tally,
                Some(format!("devin:{session}:{time}:{}", {
                    use sha2::{Digest, Sha256};
                    format!("{:x}", Sha256::digest(message.as_bytes()))
                })),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "qs-dsh-stream-{}-{}-{}",
                std::process::id(),
                crate::timeutil::now_ms(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
        fn read(&self) -> Reader {
            read_source("dsh", std::slice::from_ref(&self.0))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn total(reader: &Reader) -> i64 {
        reader
            .buckets
            .values()
            .flat_map(|models| models.values())
            .map(TokenTally::total)
            .sum()
    }

    #[test]
    fn zcode_reads_the_fixed_database_and_keeps_standard_imports_without_scanning_rollouts() {
        let fixture = Fixture::new();
        let native = fixture.0.join("cli/db/db.sqlite");
        std::fs::create_dir_all(native.parent().unwrap()).unwrap();
        let db = rusqlite::Connection::open(&native).unwrap();
        db.execute_batch("CREATE TABLE model_usage (
            id TEXT PRIMARY KEY, logical_request_id TEXT, attempt_index INTEGER, model_id TEXT,
            status TEXT, started_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
            reasoning_tokens INTEGER, cache_creation_input_tokens INTEGER, cache_read_input_tokens INTEGER,
            computed_total_tokens INTEGER, provider_total_tokens INTEGER, raw_usage_json TEXT);
            CREATE INDEX model_usage_started_model_idx ON model_usage(started_at,model_id);").unwrap();
        db.execute(
            "INSERT INTO model_usage VALUES ('one','logical',0,'fixture','completed',?1,
            100,20,5,10,30,120,120,?2)",
            rusqlite::params![1790850000000_i64,
            json!({"inputTokens":100,"outputTokens":20,"reasoningTokens":5,"cacheWriteTokens":10,
                "cacheReadTokens":30,"totalTokens":120}).to_string()],
        )
        .unwrap();
        drop(db);
        fixture.write("should-not-read-rollout.jsonl", b"malformed conversation\n");
        let first = read_source("zcode", &[fixture.0.clone(), native.clone()]);
        assert_eq!(total(&first), 120);
        assert!(!first.partial);
        assert!(native_supported("zcode"));
        let imports = fixture.0.join("UsageImports/zcode");
        std::fs::create_dir_all(&imports).unwrap();
        std::fs::write(
            imports.join("import.jsonl"),
            json!({"schema":"quotascope.usage.v1",
            "source":"zcode","id":"import-one","timestamp":1790850000000_i64,"model":"fixture",
            "usage":{"inputTokens":2,"outputTokens":3}})
            .to_string()
                + "\n",
        )
        .unwrap();
        let combined = read_source("zcode", &[native.clone(), imports]);
        assert_eq!(total(&combined), 125);
        assert!(!combined.partial);
        let copy = fixture.0.join("copy/db.sqlite");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::copy(&native, &copy).unwrap();
        let multiple = read_source("zcode", &[native, copy]);
        assert_eq!(total(&multiple), 120);
        assert!(multiple.partial);
    }

    #[test]
    fn zcode_import_depth_limits_are_visible_instead_of_silently_complete() {
        let fixture = Fixture::new();
        let imports = fixture.0.join("UsageImports/zcode");
        let deep = (0..10).fold(imports.clone(), |path, _| path.join("nested"));
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("usage.jsonl"), "{}\n").unwrap();
        assert!(read_source("zcode", &[imports]).partial);
    }
    fn transcript() -> Vec<u8> {
        [
            json!({"type":"session","seedLength":3}),
            json!({"type":"request/header","data":{"header":{"config":{"model":"fixture"}}}}),
            json!({"type":"assistant/chunk","seq":3,"time":1790850000000_i64,"data":{"usage":{"inputTokens":9999}}}),
            json!({"type":"compaction/summary","seq":4,"time":1790850000000_i64,"data":{"compactionId":"compact","usage":{"inputTokens":9999}}}),
            json!({"type":"assistant/message","seq":2,"time":1790850000000_i64,"data":{"message":{"id":"inherited"},"usage":{"inputTokens":9999}}}),
            json!({"type":"assistant/message","seq":3,"time":1790850000000_i64,"data":{"message":{"id":"one"},"usage":{"inputTokens":10,"outputTokens":8,"cacheReadTokens":2,"reasoningTokens":5}}}),
        ].into_iter().map(|row| row.to_string()+"\n").collect::<String>().into_bytes()
    }

    #[test]
    fn real_frames_and_plain_zstd_suffixes_share_identity_and_keep_assistant_only_semantics() {
        let fixture = Fixture::new();
        let plain = transcript();
        fixture.write("session.jsonl.zstd", &plain);
        let first = fixture.read();
        assert_eq!(total(&first), 20);
        assert!(!first.partial);
        fixture.write(
            "session.v3.jsonl.zstd",
            &zstd::stream::encode_all(&plain[..], 1).unwrap(),
        );
        let mut fork = String::from_utf8(plain.clone())
            .unwrap()
            .replace("\"seedLength\":3", "\"seedLength\":4");
        fork.push_str(&(json!({"type":"assistant/message","seq":4,"time":1790850000001_i64,"data":{"message":{"id":"two"},"usage":{"inputTokens":4,"outputTokens":3}}}).to_string()+"\n"));
        fixture.write(
            "session.fork.jsonl.zstd",
            &zstd::stream::encode_all(fork.as_bytes(), 1).unwrap(),
        );
        let combined = fixture.read();
        assert_eq!(total(&combined), 27);
        assert!(!combined.partial);
    }

    #[test]
    fn concatenated_append_truncation_and_repair_report_readable_prefixes_as_partial() {
        use std::io::Write;
        let fixture = Fixture::new();
        let prefix = zstd::stream::encode_all(&transcript()[..], 1).unwrap();
        let tail=json!({"type":"assistant/message","seq":4,"time":1790850000001_i64,"data":{"message":{"id":"two"},"usage":{"inputTokens":4,"outputTokens":3}}}).to_string()+"\n";
        let frame = zstd::stream::encode_all(tail.as_bytes(), 1).unwrap();
        let path = fixture.write("session.jsonl.zstd", &prefix);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&frame)
            .unwrap();
        let whole = fixture.read();
        assert_eq!(total(&whole), 27);
        assert!(!whole.partial);
        let mut truncated = prefix.clone();
        truncated.extend(&frame[..frame.len() / 2]);
        std::fs::write(&path, &truncated).unwrap();
        let torn = fixture.read();
        assert!(torn.partial && total(&torn) >= 20 && total(&torn) <= 27);
        let mut corrupt = prefix.clone();
        corrupt.extend([0x28, 0xb5, 0x2f, 0xfd, 0xff, 0xff]);
        std::fs::write(&path, &corrupt).unwrap();
        assert!(fixture.read().partial);
        std::fs::write(&path, &prefix).unwrap();
        let repaired = fixture.read();
        assert_eq!(total(&repaired), 20);
        assert!(!repaired.partial);
    }

    #[test]
    fn oversized_decoded_line_is_partial_without_parsing_its_tail() {
        let fixture = Fixture::new();
        let mut text = transcript();
        text.extend(std::iter::repeat_n(
            b'x',
            crate::zstd_stream::MAX_LINE_BYTES + 1,
        ));
        text.push(b'\n');
        text.extend(transcript());
        fixture.write(
            "session.jsonl.zstd",
            &zstd::stream::encode_all(&text[..], 1).unwrap(),
        );
        let bounded = fixture.read();
        assert!(bounded.partial);
        assert_eq!(total(&bounded), 20);
    }

    #[test]
    fn cancellation_during_decompression_exits_and_does_not_poison_a_fresh_scan() {
        use std::io::Write;
        let fixture = Fixture::new();
        let path = fixture.0.join("session.jsonl.zstd");
        let mut encoded =
            zstd::stream::Encoder::new(std::fs::File::create(&path).unwrap(), 1).unwrap();
        encoded.write_all(&transcript()).unwrap();
        let noise = json!({"type":"tool/result","data":{"text":"ignored"}}).to_string() + "\n";
        for _ in 0..300000 {
            encoded.write_all(noise.as_bytes()).unwrap();
        }
        encoded.finish().unwrap();
        let control = std::sync::Arc::new(crate::scan::Control::default());
        let stop = control.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            stop.cancel();
        });
        let began = std::time::Instant::now();
        let result = crate::scan::run(control, 1, |_| {}, || fixture.read());
        thread.join().unwrap();
        assert!(result.is_err());
        assert!(began.elapsed() < std::time::Duration::from_secs(5));
        let fresh = fixture.read();
        assert!(!fresh.partial);
        assert_eq!(total(&fresh), 20);
    }

    #[test]
    fn dsh_missing_or_invalid_main_counters_are_not_filled_with_zero() {
        let mut reader = Reader::default();
        let file = Path::new("session.jsonl");
        let mut context = (0, Some("fixture".into()));
        for usage in [
            json!({"inputTokens":5}),
            json!({"inputTokens":5,"outputTokens":-1}),
            json!({"inputTokens":5.5,"outputTokens":2}),
            json!({"inputTokens":5,"outputTokens":2,"cacheReadTokens":null}),
        ] {
            parse(
                "dsh",
                &json!({"type":"assistant/message","seq":1,"time":1790850000000_i64,"data":{"usage":usage}}),
                false,
                file,
                &mut reader,
                &mut context,
            );
        }
        assert!(reader.partial && reader.buckets.is_empty());
        let mut unidentified = Reader::default();
        parse(
            "dsh",
            &json!({"type":"assistant/message","seq":1,"time":1790850000000_i64,"data":{"usage":{"inputTokens":5,"outputTokens":2}}}),
            false,
            file,
            &mut unidentified,
            &mut (0, None),
        );
        assert!(unidentified.partial && unidentified.buckets.is_empty());
        assert_eq!(
            dsh_tally(&json!({"inputTokens":0,"outputTokens":0}))
                .unwrap()
                .total(),
            0
        );
    }
    #[test]
    fn dsh_fork_seed_and_reasoning_subset_do_not_double_count() {
        let mut reader = Reader::default();
        let mut context = (0, None);
        let file = Path::new("session.jsonl");
        parse(
            "dsh",
            &json!({"type":"session","seedLength":3}),
            false,
            file,
            &mut reader,
            &mut context,
        );
        parse(
            "dsh",
            &json!({"type":"request/header","data":{"header":{"config":{"model":"test"}}}}),
            false,
            file,
            &mut reader,
            &mut context,
        );
        let mut row = json!({"type":"assistant/message","seq":2,"time":1790850000000_i64,"data":{"message":{"id":"one"},"usage":{"inputTokens":10,"outputTokens":8,"reasoningTokens":5}}});
        parse("dsh", &row, false, file, &mut reader, &mut context);
        assert!(reader.buckets.is_empty());
        row["seq"] = json!(3);
        parse("dsh", &row, false, file, &mut reader, &mut context);
        parse("dsh", &row, false, file, &mut reader, &mut context);
        assert_eq!(reader.buckets.values().next().unwrap()["test"].total(), 18);
    }
    #[test]
    fn imports_require_source_schema_and_stable_identity() {
        let mut reader = Reader::default();
        let file = Path::new("export.jsonl");
        parse(
            "zed",
            &json!({"tokens":999}),
            true,
            file,
            &mut reader,
            &mut (0, None),
        );
        assert!(reader.partial && reader.buckets.is_empty());
        let row = json!({"schema":"quotascope.usage.v1","source":"zed","id":"request","timestamp":1790850000000_i64,"model":"unknown","usage":{"inputTokens":10,"outputTokens":7}});
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        assert_eq!(
            reader.buckets.values().next().unwrap()["unknown"].total(),
            17
        );
    }

    fn import_row() -> Value {
        json!({"schema":"quotascope.usage.v1","source":"zed","id":"request",
            "timestamp":1790850000000_i64,"model":"fixture",
            "usage":{"inputTokens":10,"outputTokens":7,"cacheReadTokens":5,
                "cacheWriteTokens":3,"reasoningTokens":4,"totalTokens":25}})
    }

    #[test]
    fn imported_counters_reject_missing_and_conflicting_usage_instead_of_guessing() {
        for usage in [
            json!({"outputTokens":7}),
            json!({"inputTokens":10}),
            json!({"inputTokens":-1,"outputTokens":7}),
            json!({"inputTokens":10.5,"outputTokens":7}),
            json!({"inputTokens":"10","outputTokens":7}),
            json!({"inputTokens":true,"outputTokens":7}),
            json!({"inputTokens":1_000_000_000_001_i64,"outputTokens":7}),
            json!({"inputTokens":10,"outputTokens":7,"cacheReadTokens":null}),
            json!({"inputTokens":10,"outputTokens":7,"cacheWriteTokens":-1}),
            json!({"inputTokens":10,"outputTokens":7,"reasoningTokens":8}),
            json!({"inputTokens":10,"outputTokens":7,"reasoningTokens":null}),
            json!({"inputTokens":10,"outputTokens":7,"totalTokens":18}),
            json!({"inputTokens":10,"outputTokens":7,"totalTokens":"17"}),
            json!({"inputTokens":10,"outputTokens":7,"totalTokens":null}),
        ] {
            let mut row = import_row();
            row["usage"] = usage;
            let mut reader = Reader::default();
            parse(
                "zed",
                &row,
                true,
                Path::new("usage.jsonl"),
                &mut reader,
                &mut (0, None),
            );
            assert!(reader.partial && reader.buckets.is_empty(), "{row}");
        }
    }

    #[test]
    fn imported_timestamp_and_identity_must_define_an_actual_hourly_record() {
        for timestamp in [
            Value::Null,
            json!(0),
            json!(i64::MAX),
            json!(1790850000000.5),
            json!("1790850000000"),
            json!("2026-10-01"),
            json!("2026-10-01T14:00:00"),
            json!(chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp_millis()),
        ] {
            let mut row = import_row();
            row["timestamp"] = timestamp;
            let mut reader = Reader::default();
            parse(
                "zed",
                &row,
                true,
                Path::new("usage.jsonl"),
                &mut reader,
                &mut (0, None),
            );
            assert!(reader.partial && reader.buckets.is_empty(), "{row}");
        }
        for identity in [json!(""), json!(" \t"), json!("x".repeat(1025)), json!(7)] {
            let mut row = import_row();
            row["id"] = identity;
            let mut reader = Reader::default();
            parse(
                "zed",
                &row,
                true,
                Path::new("usage.jsonl"),
                &mut reader,
                &mut (0, None),
            );
            assert!(reader.partial && reader.buckets.is_empty());
        }
        assert_eq!(
            import_time(&json!("2026-10-01T14:00:00+08:00")),
            import_time(&json!("2026-10-01T06:00:00Z"))
        );
        assert!(import_time(&json!("2026-10-01T06:00:00Z")).is_some());
    }

    #[test]
    fn explicit_zero_snapshot_removes_previous_import_and_a_later_update_replaces_it() {
        let mut reader = Reader::default();
        let mut row = import_row();
        let file = Path::new("usage.jsonl");
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        assert_eq!(total(&reader), 25);
        row["usage"] = json!({"inputTokens":0,"outputTokens":0});
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        assert_eq!(total(&reader), 0);
        assert!(reader.buckets.is_empty());
        row["usage"] = json!({"inputTokens":2,"outputTokens":3});
        row["model"] = json!("updated");
        row["timestamp"] = json!(1790936400000_i64);
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        assert_eq!(total(&reader), 5);
        assert!(!reader.partial);
        assert_eq!(reader.ids.len(), 1);
        assert_eq!(reader.buckets.len(), 1);
        assert_eq!(reader.buckets.values().next().unwrap().len(), 1);
    }

    #[test]
    fn imported_jsonl_keeps_valid_usage_unpriced_and_reports_invalid_records_as_partial() {
        let fixture = Fixture::new();
        let imports = fixture.0.join("UsageImports/zed");
        std::fs::create_dir_all(&imports).unwrap();
        let known = import_row();
        let mut unknown = import_row();
        unknown["id"] = json!("unknown-model");
        unknown.as_object_mut().unwrap().remove("model");
        unknown["usage"] = json!({"inputTokens":2,"outputTokens":3});
        let mut invalid = import_row();
        invalid["id"] = json!("invalid");
        invalid["usage"] = json!({"inputTokens":999});
        let content = [known, unknown, invalid]
            .into_iter()
            .map(|row| row.to_string() + "\n")
            .collect::<String>();
        std::fs::write(imports.join("usage.jsonl"), content).unwrap();
        let reader = read_source("zed", std::slice::from_ref(&imports));
        assert_eq!(total(&reader), 30);
        let models = reader.buckets.values().next().unwrap();
        assert_eq!(
            models["fixture"],
            TokenTally {
                input: 10,
                output: 7,
                cache_read: 5,
                cache_write: 3
            }
        );
        assert_eq!(models["unknown"].total(), 5);
        let ledger = read_with_prices("zed", &[imports], &BTreeMap::new());
        assert!(ledger.has_partial_records);
        assert_eq!(ledger.days.iter().map(|day| day.tokens).sum::<i64>(), 30);
        assert_eq!(
            ledger
                .days
                .iter()
                .map(|day| day.unpriced_tokens)
                .sum::<i64>(),
            30
        );
    }

    #[test]
    fn import_snapshot_order_is_stable_across_root_order_and_model_changes() {
        let fixture = Fixture::new();
        let imports = fixture.0.join("UsageImports/zed");
        std::fs::create_dir_all(&imports).unwrap();
        let first = imports.join("01-old.json");
        let last = imports.join("02-new.json");
        let old = import_row();
        let mut new = import_row();
        new["model"] = json!("replacement");
        new["usage"] = json!({"inputTokens":2,"outputTokens":3});
        std::fs::write(&first, old.to_string()).unwrap();
        std::fs::write(&last, new.to_string()).unwrap();
        for paths in [vec![first.clone(), last.clone()], vec![last, first]] {
            let reader = read_source("zed", &paths);
            assert_eq!(total(&reader), 5);
            assert!(!reader.partial);
            let models = reader.buckets.values().next().unwrap();
            assert_eq!(models.len(), 1);
            assert!(models.contains_key("replacement"));
        }
    }

    #[test]
    fn import_stream_byte_limit_is_shared_and_exact_eof_remains_complete() {
        let bytes = Rc::new(Cell::new(0));
        let mut first = ImportInput {
            inner: &b"abc"[..],
            bytes: bytes.clone(),
            maximum: 5,
        };
        let mut output = String::new();
        first.read_to_string(&mut output).unwrap();
        assert_eq!(output, "abc");
        let mut second = ImportInput {
            inner: &b"de"[..],
            bytes: bytes.clone(),
            maximum: 5,
        };
        second.read_to_string(&mut output).unwrap();
        assert_eq!(output, "abcde");
        let mut excessive = ImportInput {
            inner: &b"ignored"[..],
            bytes: bytes.clone(),
            maximum: 5,
        };
        assert!(excessive.read_to_string(&mut output).is_err());
        assert_eq!(output, "abcde");
        assert_eq!(bytes.get(), 6, "one byte probes beyond the exact budget");
        assert!(excessive.read_to_string(&mut output).is_err());
        assert_eq!(bytes.get(), 6, "exhausted streams stop reading");
    }
}
