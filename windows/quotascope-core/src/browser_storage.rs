//! Read-only, bounded Chromium localStorage. Only active manifest files and
//! the requested origin/keys are decoded; newer deletions suppress old tokens.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
type Values = BTreeMap<Vec<u8>, (u64, Option<Vec<u8>>)>;
pub fn windsurf() -> Option<(String, String)> {
    let keys = [
        "devin_session_token",
        "devin_auth1_token",
        "devin_account_id",
        "devin_primary_org_id",
    ];
    let (values, browser) = find("https://windsurf.com", &keys, |values| {
        keys.iter()
            .all(|key| values.get(*key).is_some_and(|v| !v.is_empty()))
    })?;
    Some((serde_json::to_string(&values).ok()?, browser))
}

/// Default profile first, followed by a bounded, deterministic set of named
/// profiles. The underlying reader still selects only this origin and keys.
pub fn find(
    origin: &str,
    keys: &[&str],
    accept: impl Fn(&BTreeMap<String, String>) -> bool,
) -> Option<(BTreeMap<String, String>, String)> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    for (browser, relative) in [
        ("Edge", "Microsoft/Edge/User Data"),
        ("Chrome", "Google/Chrome/User Data"),
        ("Brave", "BraveSoftware/Brave-Browser/User Data"),
        ("Vivaldi", "Vivaldi/User Data"),
    ] {
        let root = std::path::PathBuf::from(&local).join(relative);
        let mut named: Vec<_> = std::fs::read_dir(&root)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .take(256)
            .filter(|profile| {
                profile
                    .file_name()
                    .to_string_lossy()
                    .starts_with("Profile ")
            })
            .map(|profile| profile.path())
            .collect();
        named.sort();
        named.truncate(31);
        for profile in std::iter::once(root.join("Default")).chain(named) {
            let path = profile.join("Local Storage/leveldb");
            if let Some(values) = read(&path, origin, keys) {
                if accept(&values) {
                    return Some((values, browser.into()));
                }
            }
        }
    }
    None
}
pub fn read(path: &Path, origin: &str, keys: &[&str]) -> Option<BTreeMap<String, String>> {
    let current = std::fs::read_to_string(path.join("CURRENT")).ok()?;
    let current = current.trim();
    if !current.starts_with("MANIFEST-") || current.contains(['/', '\\']) {
        return None;
    }
    let before = bounded(&path.join(current))?;
    let mut active = BTreeSet::new();
    let mut log = None;
    let mut previous_log = None;
    for record in records(&before)? {
        let mut at = 0;
        while at < record.len() {
            match var(&record, &mut at)? {
                1 => {
                    let _ = slice(&record, &mut at)?;
                }
                2 => {
                    log = Some(var(&record, &mut at)?);
                }
                3 | 4 => {
                    let _ = var(&record, &mut at)?;
                }
                5 => {
                    let _ = var(&record, &mut at)?;
                    let _ = slice(&record, &mut at)?;
                }
                6 => {
                    let _ = var(&record, &mut at)?;
                    active.remove(&var(&record, &mut at)?);
                }
                7 => {
                    let _ = var(&record, &mut at)?;
                    let file = var(&record, &mut at)?;
                    let _ = var(&record, &mut at)?;
                    let _ = slice(&record, &mut at)?;
                    let _ = slice(&record, &mut at)?;
                    active.insert(file);
                }
                9 => {
                    previous_log = Some(var(&record, &mut at)?);
                }
                _ => return None,
            }
        }
    }
    let mut values = Values::new();
    let prefix = format!("_{origin}\0").into_bytes();
    let wanted = |key: &[u8]| {
        key.starts_with(&prefix)
            && decode(&key[prefix.len()..]).is_some_and(|key| keys.contains(&key.as_str()))
    };
    for file in active {
        let data = bounded(&path.join(format!("{file:06}.ldb")))
            .or_else(|| bounded(&path.join(format!("{file:06}.sst"))))?;
        for (key, value) in table(&data)? {
            let end = key.len().checked_sub(8)?;
            let tag = u64::from_le_bytes(key[end..].try_into().ok()?);
            let key = &key[..end];
            if wanted(key) {
                set(
                    &mut values,
                    key.to_vec(),
                    tag >> 8,
                    if tag & 255 == 1 { Some(value) } else { None },
                );
            }
        }
    }
    for file in log.into_iter().chain(previous_log).collect::<BTreeSet<_>>() {
        let Some(data) = bounded(&path.join(format!("{file:06}.log"))) else {
            continue;
        };
        for record in records(&data)? {
            batch(&record, &mut values, &wanted)?;
        }
        if bounded(&path.join(format!("{file:06}.log")))? != data {
            return None;
        }
    }
    // A concurrent compaction changes active-file membership. Reject this
    // read instead of mixing two versions of the browser's credential store.
    if bounded(&path.join(current))? != before
        || std::fs::read_to_string(path.join("CURRENT")).ok()?.trim() != current
    {
        return None;
    }
    let mut result = BTreeMap::new();
    for (key, (_, value)) in values {
        if let Some(value) = value {
            result.insert(decode(&key[prefix.len()..])?, decode(&value)?);
        }
    }
    Some(result)
}
fn bounded(path: &Path) -> Option<Vec<u8>> {
    let input = std::fs::File::open(path).ok()?;
    let mut data = Vec::new();
    input
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() <= 64 * 1024 * 1024).then_some(data)
}
fn decode(bytes: &[u8]) -> Option<String> {
    match bytes.split_first()? {
        (1, text) => String::from_utf8(text.to_vec()).ok(),
        (0, text) => {
            if text.len() % 2 != 0 {
                return None;
            }
            String::from_utf16(
                &text
                    .chunks_exact(2)
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect::<Vec<_>>(),
            )
            .ok()
        }
        _ => None,
    }
}
fn set(values: &mut Values, key: Vec<u8>, seq: u64, value: Option<Vec<u8>>) {
    if values.get(&key).is_none_or(|(old, _)| seq >= *old) {
        values.insert(key, (seq, value));
    }
}
fn var(data: &[u8], at: &mut usize) -> Option<u64> {
    let mut n = 0;
    for shift in (0..64).step_by(7) {
        let b = *data.get(*at)?;
        *at += 1;
        if shift == 63 && b > 1 {
            return None;
        }
        n |= u64::from(b & 127) << shift;
        if b & 128 == 0 {
            return Some(n);
        }
    }
    None
}
fn slice<'a>(data: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let len = usize::try_from(var(data, at)?).ok()?;
    let end = at.checked_add(len)?;
    let bytes = data.get(*at..end)?;
    *at = end;
    Some(bytes)
}
fn crc(data: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for b in data {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0x82f63b78 } else { 0 };
        }
    }
    !crc
}
fn masked(data: &[u8]) -> u32 {
    crc(data).rotate_right(15).wrapping_add(0xa282ead8)
}
fn records(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut pending = Vec::new();
    for block in data.chunks(32768) {
        let mut at = 0;
        while at + 7 <= block.len() {
            let checksum = u32::from_le_bytes(block[at..at + 4].try_into().ok()?);
            let len = u16::from_le_bytes(block[at + 4..at + 6].try_into().ok()?) as usize;
            let kind = block[at + 6];
            if kind == 0 && len == 0 {
                break;
            }
            let end = at.checked_add(7 + len)?;
            let bytes = block.get(at + 7..end)?;
            let mut checked = vec![kind];
            checked.extend(bytes);
            if masked(&checked) != checksum {
                return None;
            }
            match kind {
                1 => {
                    if !pending.is_empty() {
                        return None;
                    }
                    out.push(bytes.to_vec());
                }
                2 => {
                    if !pending.is_empty() {
                        return None;
                    }
                    pending = bytes.to_vec();
                }
                3 => {
                    if pending.is_empty() {
                        return None;
                    }
                    pending.extend(bytes);
                }
                4 => {
                    if pending.is_empty() {
                        return None;
                    }
                    pending.extend(bytes);
                    out.push(std::mem::take(&mut pending));
                }
                _ => return None,
            }
            if pending.len() > 64 * 1024 * 1024 {
                return None;
            }
            at = end;
        }
        if block[at..].iter().any(|b| *b != 0) {
            return None;
        }
    }
    if !pending.is_empty() {
        return None;
    }
    Some(out)
}
fn batch(record: &[u8], values: &mut Values, wanted: &impl Fn(&[u8]) -> bool) -> Option<()> {
    let seq = u64::from_le_bytes(record.get(..8)?.try_into().ok()?);
    let count = u32::from_le_bytes(record.get(8..12)?.try_into().ok()?);
    let mut at = 12;
    for index in 0..count {
        let kind = *record.get(at)?;
        at += 1;
        let key = slice(record, &mut at)?;
        let value = match kind {
            0 => None,
            1 => Some(slice(record, &mut at)?.to_vec()),
            _ => return None,
        };
        if wanted(key) {
            set(
                values,
                key.to_vec(),
                seq.checked_add(u64::from(index))?,
                value,
            );
        }
    }
    (at == record.len()).then_some(())
}
fn block(data: &[u8], handle: &[u8]) -> Option<Vec<u8>> {
    let mut at = 0;
    let start = usize::try_from(var(handle, &mut at)?).ok()?;
    let len = usize::try_from(var(handle, &mut at)?).ok()?;
    let end = start.checked_add(len)?;
    let raw = data.get(start..end)?;
    let kind = *data.get(end)?;
    let expected = u32::from_le_bytes(data.get(end + 1..end + 5)?.try_into().ok()?);
    let mut checked = raw.to_vec();
    checked.push(kind);
    if masked(&checked) != expected {
        return None;
    }
    match kind {
        0 => Some(raw.to_vec()),
        1 => {
            if snap::raw::decompress_len(raw).ok()? > 64 * 1024 * 1024 {
                return None;
            }
            snap::raw::Decoder::new().decompress_vec(raw).ok()
        }
        _ => None,
    }
}
fn entries(data: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    let count =
        u32::from_le_bytes(data.get(data.len().checked_sub(4)?..)?.try_into().ok()?) as usize;
    let end = data
        .len()
        .checked_sub(count.checked_mul(4)?.checked_add(4)?)?;
    let mut at = 0;
    let mut previous = Vec::new();
    let mut out = Vec::new();
    while at < end {
        let shared = usize::try_from(var(data, &mut at)?).ok()?;
        let fresh = usize::try_from(var(data, &mut at)?).ok()?;
        let len = usize::try_from(var(data, &mut at)?).ok()?;
        let mut key = previous.get(..shared)?.to_vec();
        let next = at.checked_add(fresh)?;
        key.extend(data.get(at..next)?);
        at = next;
        let next = at.checked_add(len)?;
        let value = data.get(at..next)?.to_vec();
        at = next;
        if at > end {
            return None;
        }
        previous = key.clone();
        out.push((key, value));
    }
    Some(out)
}
fn table(data: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    if data.len() < 48
        || u64::from_le_bytes(data[data.len() - 8..].try_into().ok()?) != 0xdb4775248b80fb57
    {
        return None;
    }
    let footer = &data[data.len() - 48..];
    let mut at = 0;
    let _ = var(footer, &mut at)?;
    let _ = var(footer, &mut at)?;
    let index = block(data, &footer[at..])?;
    let mut out = Vec::new();
    for (_, handle) in entries(&index)? {
        out.extend(entries(&block(data, &handle)?)?);
    }
    Some(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn newer_delete_never_resurrects_saved_auth() {
        let key = b"_https://windsurf.com\0\x01devin_session_token".to_vec();
        let mut values = Values::new();
        set(&mut values, key.clone(), 10, Some(b"\x01old".to_vec()));
        set(&mut values, key.clone(), 11, None);
        set(&mut values, key.clone(), 9, Some(b"\x01stale".to_vec()));
        assert!(values[&key].1.is_none());
        assert_eq!(decode(b"\0t\0o\0k\0e\0n\0"), Some("token".into()));
    }
    #[test]
    fn checksums_and_incomplete_fragments_fail_closed() {
        assert_eq!(crc(b"123456789"), 0xe3069283);
        assert!(records(&[1, 0, 0, 0, 1, 0, 1, 9]).is_none());
        assert!(table(&[0; 48]).is_none());
        assert!(records(&[1, 2, 3]).is_none());
    }
    #[test]
    fn active_manifest_log_filters_origins_and_obsolete_credentials() {
        fn record(data: &[u8]) -> Vec<u8> {
            let mut checked = vec![1];
            checked.extend(data);
            let mut bytes = masked(&checked).to_le_bytes().to_vec();
            bytes.extend((data.len() as u16).to_le_bytes());
            bytes.push(1);
            bytes.extend(data);
            bytes
        }
        fn batch_bytes(sequence: u64, origin: &str, token: &str) -> Vec<u8> {
            let key = format!("_{origin}\0\x01devin_session_token").into_bytes();
            let value = format!("\x01{token}").into_bytes();
            let mut data = sequence.to_le_bytes().to_vec();
            data.extend(1u32.to_le_bytes());
            data.push(1);
            data.push(key.len() as u8);
            data.extend(key);
            data.push(value.len() as u8);
            data.extend(value);
            record(&data)
        }
        let path = std::env::temp_dir().join(format!(
            "qs-storage-{}-{}",
            std::process::id(),
            crate::timeutil::now_ms()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("CURRENT"), "MANIFEST-000001\n").unwrap();
        std::fs::write(path.join("MANIFEST-000001"), record(&[2, 2, 9, 0])).unwrap();
        std::fs::write(
            path.join("000003.log"),
            batch_bytes(99, "https://windsurf.com", "obsolete"),
        )
        .unwrap();
        let mut live = batch_bytes(10, "https://windsurf.com", "live");
        live.extend(batch_bytes(11, "https://other.example", "unrelated"));
        std::fs::write(path.join("000002.log"), &live).unwrap();
        let found = read(&path, "https://windsurf.com", &["devin_session_token"]).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found["devin_session_token"], "live");
        live.extend([1, 2, 3]);
        std::fs::write(path.join("000002.log"), live).unwrap();
        assert!(read(&path, "https://windsurf.com", &["devin_session_token"]).is_none());
        std::fs::remove_dir_all(path).unwrap();
    }
}
