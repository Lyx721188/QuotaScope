//! Cooperative cancellation and measured progress for one local scan thread.
//! Other readers keep their ordinary behavior, including other UI workers.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Control {
    cancelled: AtomicBool,
}

impl Control {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub source: String,
    pub sources_done: usize,
    pub sources_total: usize,
    pub files_read: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Cancelled;

struct Context {
    control: Arc<Control>,
    progress: Progress,
    sink: Rc<dyn Fn(Progress)>,
    emitted_at: Instant,
}

thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

struct Restore(Option<Context>);
impl Drop for Restore {
    fn drop(&mut self) {
        CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

pub fn run<T>(
    control: Arc<Control>,
    sources_total: usize,
    sink: impl Fn(Progress) + 'static,
    read: impl FnOnce() -> T,
) -> Result<T, Cancelled> {
    let previous = CURRENT.with(|slot| {
        slot.replace(Some(Context {
            control,
            progress: Progress {
                sources_total,
                ..Default::default()
            },
            sink: Rc::new(sink),
            emitted_at: Instant::now(),
        }))
    });
    let _restore = Restore(previous);
    if !checkpoint() {
        return Err(Cancelled);
    }
    let value = read();
    if checkpoint() {
        Ok(value)
    } else {
        Err(Cancelled)
    }
}

pub(crate) fn checkpoint() -> bool {
    CURRENT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_none_or(|c| !c.control.is_cancelled())
    })
}

fn notify(change: impl FnOnce(&mut Progress), force: bool) {
    let notification = CURRENT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot.as_mut()?;
        if context.control.is_cancelled() {
            return None;
        }
        change(&mut context.progress);
        if !force && context.emitted_at.elapsed() < Duration::from_millis(200) {
            return None;
        }
        context.emitted_at = Instant::now();
        Some((context.sink.clone(), context.progress.clone()))
    });
    // Release the thread-local borrow before invoking application code.
    if let Some((sink, progress)) = notification {
        sink(progress);
    }
}

pub(crate) fn source_started(name: &str) {
    notify(|p| p.source = name.to_owned(), true);
}

pub(crate) fn source_finished() {
    notify(|p| p.sources_done += 1, true);
}

fn read_chunks(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 64 * 1024];
    loop {
        if !checkpoint() {
            return Err(io::Error::other("scan cancelled"));
        }
        let count = match reader.read(&mut chunk) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if !checkpoint() {
        return Err(io::Error::other("scan cancelled"));
    }
    Ok(bytes)
}

pub(crate) fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    if CURRENT.with(|slot| slot.borrow().is_none()) {
        return std::fs::read(path);
    }
    if !checkpoint() {
        return Err(io::Error::other("scan cancelled"));
    }
    let bytes = read_chunks(std::fs::File::open(path)?)?;
    notify(|p| p.files_read += 1, false);
    Ok(bytes)
}

pub(crate) fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Check between buffered disk reads, including inside a very long line.
struct CheckedReader<R>(R);
impl<R: Read> Read for CheckedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !checkpoint() {
            return Err(io::Error::other("scan cancelled"));
        }
        self.0.read(buffer)
    }
}

/// Retains one line and a 64 KiB buffer, rather than the entire transcript.
/// Errors are kept for the caller: a partially read file is never cached.
pub(crate) struct LineReader<R> {
    reader: BufReader<CheckedReader<R>>,
    digest: DefaultHasher,
    offset: u64,
    ends_with_newline: bool,
    error: Option<io::Error>,
    done: bool,
    max_line_bytes: Option<usize>,
}

impl<R: Read> LineReader<R> {
    pub(crate) fn new(reader: R, digest: DefaultHasher, offset: u64) -> Self {
        Self {
            reader: BufReader::with_capacity(64 * 1024, CheckedReader(reader)),
            digest,
            offset,
            ends_with_newline: offset == 0,
            error: None,
            done: false,
            max_line_bytes: None,
        }
    }

    pub(crate) fn with_line_limit(reader: R, maximum: usize) -> Self {
        let mut lines = Self::new(reader, DefaultHasher::new(), 0);
        lines.max_line_bytes = Some(maximum);
        lines
    }

    pub(crate) fn finish(&mut self) -> io::Result<(u64, u64, bool)> {
        if let Some(error) = &self.error {
            return Err(io::Error::new(error.kind(), error.to_string()));
        }
        if !self.done || !checkpoint() {
            return Err(io::Error::other("incomplete scan"));
        }
        Ok((self.offset, self.digest.finish(), self.ends_with_newline))
    }
}

impl<R: Read> Iterator for LineReader<R> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if self.done {
            return None;
        }
        let mut bytes = Vec::new();
        let result = if checkpoint() {
            if let Some(maximum) = self.max_line_bytes {
                let read = self
                    .reader
                    .by_ref()
                    .take(maximum.saturating_add(1) as u64)
                    .read_until(b'\n', &mut bytes);
                if bytes.len() > maximum {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "JSONL line exceeds the read limit",
                    ))
                } else {
                    read
                }
            } else {
                self.reader.read_until(b'\n', &mut bytes)
            }
        } else {
            Err(io::Error::other("scan cancelled"))
        };
        match result {
            Ok(0) => {
                self.done = true;
                notify(|p| p.files_read += 1, false);
                None
            }
            Ok(count) => {
                self.offset += count as u64;
                self.digest.write(&bytes);
                self.ends_with_newline = bytes.last() == Some(&b'\n');
                if self.ends_with_newline {
                    bytes.pop();
                    if bytes.last() == Some(&b'\r') {
                        bytes.pop();
                    }
                }
                match String::from_utf8(bytes) {
                    Ok(line) => Some(line),
                    Err(error) => {
                        self.error = Some(io::Error::new(io::ErrorKind::InvalidData, error));
                        self.done = true;
                        None
                    }
                }
            }
            Err(error) => {
                self.error = Some(error);
                self.done = true;
                None
            }
        }
    }
}

/// Re-read the old prefix without parsing JSON. A mismatch forces a full scan.
/// This digest guards derived-cache reuse; it is not a security checksum.
pub(crate) fn verify_prefix(
    reader: &mut impl Read,
    mut remaining: u64,
    expected: u64,
) -> io::Result<Option<DefaultHasher>> {
    let mut digest = DefaultHasher::new();
    let mut chunk = [0; 64 * 1024];
    while remaining != 0 {
        if !checkpoint() {
            return Err(io::Error::other("scan cancelled"));
        }
        let limit = remaining.min(chunk.len() as u64) as usize;
        let count = match reader.read(&mut chunk[..limit]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "prefix truncated",
            ));
        }
        digest.write(&chunk[..count]);
        remaining -= count as u64;
    }
    Ok((digest.finish() == expected).then_some(digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn bounded_lines_fail_instead_of_accepting_a_truncated_record() {
        let mut lines = LineReader::with_line_limit(&b"ok\n1234567890123456\nnext\n"[..], 8);
        assert_eq!(lines.next().as_deref(), Some("ok"));
        assert!(lines.next().is_none() && lines.finish().is_err());
        let mut exact = LineReader::with_line_limit(&b"1234567\n12345678"[..], 8);
        assert_eq!(exact.by_ref().collect::<Vec<_>>(), ["1234567", "12345678"]);
        assert!(exact.finish().is_ok());
    }

    #[test]
    fn streaming_returns_the_first_line_without_loading_a_large_file() {
        struct VirtualLog {
            read: Rc<Cell<usize>>,
            remaining: usize,
        }
        impl Read for VirtualLog {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = self.remaining.min(buffer.len());
                for (i, byte) in buffer[..count].iter_mut().enumerate() {
                    *byte = b"row\n"[(self.read.get() + i) % 4];
                }
                self.read.set(self.read.get() + count);
                self.remaining -= count;
                Ok(count)
            }
        }
        let read = Rc::new(Cell::new(0));
        let mut lines = LineReader::new(
            VirtualLog {
                read: read.clone(),
                remaining: 64 * 1024 * 1024,
            },
            DefaultHasher::new(),
            0,
        );
        assert_eq!(lines.next().as_deref(), Some("row"));
        assert!(read.get() <= 64 * 1024);
        assert!(
            lines.finish().is_err(),
            "an unfinished file cannot be cached"
        );
    }

    #[test]
    fn streaming_keeps_line_endings_and_digest_consistent_across_read_boundaries() {
        let text = format!("{}\r\n\n尾行", "record\n".repeat(12000));
        let mut lines = LineReader::new(text.as_bytes(), DefaultHasher::new(), 0);
        assert_eq!(
            lines.by_ref().collect::<Vec<_>>(),
            text.lines().map(str::to_string).collect::<Vec<_>>()
        );
        let (offset, digest, newline) = lines.finish().unwrap();
        assert_eq!(offset, text.len() as u64);
        assert!(!newline);
        assert!(verify_prefix(&mut text.as_bytes(), offset, digest)
            .unwrap()
            .is_some());
        let mut changed = text.into_bytes();
        changed[70_000] ^= 1;
        assert!(verify_prefix(&mut changed.as_slice(), offset, digest)
            .unwrap()
            .is_none());
    }

    #[test]
    fn invalid_utf8_and_read_errors_cannot_become_successful_partial_files() {
        let mut lines = LineReader::new(b"valid\n\xff\n".as_slice(), DefaultHasher::new(), 0);
        assert_eq!(lines.next().as_deref(), Some("valid"));
        assert!(lines.next().is_none());
        assert_eq!(
            lines.finish().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(lines.finish().is_err());
    }

    #[test]
    fn cancellation_also_stops_inside_a_single_large_line() {
        struct LongLine(Arc<Control>);
        impl Read for LongLine {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                assert!(!self.0.is_cancelled(), "read continued after cancellation");
                buffer.fill(b'x');
                self.0.cancel();
                Ok(buffer.len())
            }
        }
        let control = Arc::new(Control::default());
        let reader = LongLine(control.clone());
        let result = run(
            control,
            1,
            |_| {},
            || {
                let mut lines = LineReader::new(reader, DefaultHasher::new(), 0);
                assert!(lines.next().is_none());
                assert!(lines.finish().is_err());
            },
        );
        assert!(matches!(result, Err(Cancelled)));
    }

    #[test]
    fn cancellation_stops_a_large_read_before_the_next_chunk() {
        struct CancelAfterChunk {
            control: Arc<Control>,
            calls: Arc<AtomicBool>,
        }
        impl Read for CancelAfterChunk {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                assert!(
                    !self.calls.swap(true, Ordering::Relaxed),
                    "read after cancellation"
                );
                buffer.fill(b'x');
                self.control.cancel();
                Ok(buffer.len())
            }
        }
        let control = Arc::new(Control::default());
        let reader = CancelAfterChunk {
            control: control.clone(),
            calls: Arc::new(AtomicBool::new(false)),
        };
        assert!(matches!(
            run(control, 1, |_| {}, || read_chunks(reader)),
            Err(Cancelled)
        ));
        assert!(checkpoint(), "cancelled scope leaked into ordinary readers");
    }

    #[test]
    fn progress_tracks_completed_sources_and_cancellation_does_not_publish_a_finish() {
        let events = std::rc::Rc::new(RefCell::new(Vec::new()));
        let output = events.clone();
        let control = Arc::new(Control::default());
        let stop = control.clone();
        let result = run(
            control,
            2,
            move |p| output.borrow_mut().push(p),
            || {
                source_started("Codex");
                source_finished();
                source_started("Qwen Code");
                stop.cancel();
                source_finished();
            },
        );
        assert_eq!(result, Err(Cancelled));
        let events = events.borrow();
        assert_eq!(events.last().unwrap().sources_done, 1);
        assert_eq!(events.last().unwrap().sources_total, 2);
        assert_eq!(events.last().unwrap().source, "Qwen Code");
    }

    #[test]
    fn a_cancelled_scan_does_not_cancel_a_reader_on_another_thread() {
        let control = Arc::new(Control::default());
        control.cancel();
        let called = Cell::new(false);
        assert_eq!(run(control, 1, |_| {}, || called.set(true)), Err(Cancelled));
        assert!(!called.get());
        assert!(std::thread::spawn(checkpoint).join().unwrap());
    }

    #[test]
    fn panicking_reader_restores_the_thread_context() {
        assert!(std::panic::catch_unwind(|| {
            let control = Arc::new(Control::default());
            let stop = control.clone();
            let _ = run(
                control,
                1,
                |_| {},
                || {
                    stop.cancel();
                    panic!("reader failed");
                },
            );
        })
        .is_err());
        assert!(checkpoint());
    }
}
