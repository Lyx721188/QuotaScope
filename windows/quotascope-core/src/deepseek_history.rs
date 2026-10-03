//! One console history read at a time, with four minute-lived results.
//! Errors are cached too; no raw response or plaintext token is retained.
use crate::deepseek_console::{self as protocol, Range};
use crate::deepseek_session::{self as session, Route};
use crate::history::HistoryRead;
use crate::model::{AccountKey, Unavailability};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Eq)]
struct Key {
    account: String,
    credential: [u8; 32],
    currency: Option<String>,
    range: Range,
}
impl Key {
    fn new(account: &AccountKey, token: &str, currency: Option<&str>, range: Range) -> Self {
        Self {
            account: account.id(),
            credential: Sha256::digest(token.as_bytes()).into(),
            currency: currency.map(str::to_string),
            range,
        }
    }
}
struct Entry {
    key: Key,
    at: Instant,
    read: Arc<HistoryRead>,
}
#[derive(Default)]
struct State {
    entries: VecDeque<Entry>,
    running: bool,
    epoch: u64,
    completed: Option<Entry>,
    waiting: usize,
}
#[derive(Default)]
struct Cache {
    state: Mutex<State>,
    changed: Condvar,
}

impl Cache {
    fn read(&self, key: Key, produce: impl FnOnce() -> (Key, HistoryRead)) -> HistoryRead {
        let epoch = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let mut joined = false;
            let entered_epoch = state.epoch;
            loop {
                if state.epoch != entered_epoch || !crate::scan::checkpoint() {
                    return HistoryRead::Failed(Unavailability::Loading);
                }
                state
                    .entries
                    .retain(|entry| entry.at.elapsed() < Duration::from_secs(60));
                if state
                    .completed
                    .as_ref()
                    .is_some_and(|entry| entry.at.elapsed() >= Duration::from_secs(60))
                {
                    state.completed = None;
                }
                if let Some(index) = state.entries.iter().position(|entry| entry.key == key) {
                    let entry = state.entries.remove(index).expect("located entry");
                    let result = entry.read.clone();
                    state.entries.push_front(entry);
                    drop(state);
                    return (*result).clone();
                }
                // A waiter with the old token joins the read that renewed it;
                // later requests look up only the effective credential key.
                if joined {
                    if let Some(entry) = state.completed.as_ref().filter(|entry| entry.key == key) {
                        let result = entry.read.clone();
                        drop(state);
                        return (*result).clone();
                    }
                }
                if !state.running {
                    state.running = true;
                    break state.epoch;
                }
                joined = true;
                state.waiting += 1;
                state = self
                    .changed
                    .wait_timeout(state, Duration::from_millis(100))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
                state.waiting -= 1;
            }
        };
        // Keep the running slot releasable even if a decoder panics.
        let (effective_key, read) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(produce))
            .unwrap_or_else(|_| {
                (
                    key.clone(),
                    HistoryRead::Failed(Unavailability::ServerError),
                )
            });
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.running = false;
        let valid = state.epoch == epoch && crate::scan::checkpoint();
        if valid {
            let shared = Arc::new(read.clone());
            state.completed = Some(Entry {
                key,
                at: Instant::now(),
                read: shared.clone(),
            });
            state.entries.retain(|entry| entry.key != effective_key);
            state.entries.push_front(Entry {
                key: effective_key,
                at: Instant::now(),
                read: shared,
            });
            state.entries.truncate(4);
        } else {
            state.completed = None;
        }
        self.changed.notify_all();
        if valid {
            read
        } else {
            HistoryRead::Failed(Unavailability::Loading)
        }
    }

    fn forget(&self, account: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.retain(|entry| entry.key.account != account);
        state.epoch = state.epoch.wrapping_add(1);
        state.completed = None;
        self.changed.notify_all();
    }
}

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Cache::default)
}
pub fn forget(account: &AccountKey) {
    cache().forget(&account.id());
}

pub fn read(account: &AccountKey, currency: Option<&str>) -> HistoryRead {
    let Some(token) = session::token(account) else {
        return HistoryRead::NotConfigured;
    };
    let now = chrono::Local::now();
    let Some(range) = Range::new(&now) else {
        return HistoryRead::Failed(Unavailability::UnreadableReply);
    };
    let key = Key::new(account, &token, currency, range.clone());
    cache().read(key, || {
        let http = crate::http::HttpClient::new();
        let (answer, renewed) = session::renewing(
            &token,
            |current| {
                let amount = session::get(&http, Route::Amount, Some(&range), current)?;
                protocol::body(&amount)?;
                let cost = session::get(&http, Route::Cost, Some(&range), current)?;
                protocol::parse_history(&amount, &cost, currency, now)
            },
            || session::renew_from_browser(account, &token),
        );
        let effective = Key::new(
            account,
            renewed.as_deref().unwrap_or(&token),
            currency,
            range,
        );
        (effective, answer.unwrap_or_else(HistoryRead::Failed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn key() -> Key {
        Key::new(
            &AccountKey::primary(Provider::DeepSeek),
            "synthetic-token",
            None,
            Range::new(&chrono::DateTime::parse_from_rfc3339("2026-10-04T12:00:00+08:00").unwrap())
                .unwrap(),
        )
    }
    fn failed(read: &HistoryRead) -> Unavailability {
        match read {
            HistoryRead::Failed(reason) => *reason,
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn errors_expiration_revision_and_all_query_dependencies_are_cached_correctly() {
        let cache = Cache::default();
        let initial = key();
        let calls = CellCounter::default();
        for _ in 0..2 {
            let result = cache.read(initial.clone(), || {
                calls.0.fetch_add(1, Ordering::SeqCst);
                (
                    initial.clone(),
                    HistoryRead::Failed(Unavailability::SessionExpired),
                )
            });
            assert_eq!(failed(&result), Unavailability::SessionExpired);
        }
        assert_eq!(calls.0.load(Ordering::SeqCst), 1);
        let mut changed = Vec::new();
        let mut next = initial.clone();
        next.account = "deepSeek#1".into();
        changed.push(next);
        let mut next = initial.clone();
        next.credential = Sha256::digest(b"new-token").into();
        changed.push(next);
        let mut next = initial.clone();
        next.currency = Some("USD".into());
        changed.push(next);
        let mut next = initial.clone();
        next.range.today = next.range.today.succ_opt().unwrap();
        changed.push(next);
        for next in changed {
            cache.read(next.clone(), || {
                calls.0.fetch_add(1, Ordering::SeqCst);
                (
                    next.clone(),
                    HistoryRead::Failed(Unavailability::RateLimited),
                )
            });
        }
        assert_eq!(calls.0.load(Ordering::SeqCst), 5);
        assert_eq!(cache.state.lock().unwrap().entries.len(), 4);
        cache.state.lock().unwrap().entries[0].at = Instant::now() - Duration::from_secs(61);
        let last = cache.state.lock().unwrap().entries[0].key.clone();
        cache.read(last.clone(), || {
            calls.0.fetch_add(1, Ordering::SeqCst);
            (last, HistoryRead::NotConfigured)
        });
        assert_eq!(calls.0.load(Ordering::SeqCst), 6);
    }
    #[derive(Default)]
    struct CellCounter(AtomicUsize);

    #[test]
    fn concurrent_same_key_callers_share_one_read_and_a_renewed_key_reuses_it() {
        let cache = Arc::new(Cache::default());
        let initial = key();
        let mut renewed = initial.clone();
        renewed.credential = Sha256::digest(b"renewed").into();
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let first_cache = cache.clone();
        let first_key = initial.clone();
        let effective = renewed.clone();
        let count = calls.clone();
        let first = std::thread::spawn(move || {
            first_cache.read(first_key, || {
                count.fetch_add(1, Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                (effective, HistoryRead::Failed(Unavailability::RateLimited))
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let second_cache = cache.clone();
        let second_key = initial.clone();
        let count = calls.clone();
        let second = std::thread::spawn(move || {
            second_cache.read(second_key.clone(), || {
                count.fetch_add(1, Ordering::SeqCst);
                (second_key, HistoryRead::NotConfigured)
            })
        });
        let until = Instant::now() + Duration::from_secs(3);
        while cache.state.lock().unwrap().waiting == 0 {
            assert!(Instant::now() < until, "Second reader did not join");
            std::thread::yield_now();
        }
        finish_tx.send(()).unwrap();
        assert_eq!(failed(&first.join().unwrap()), Unavailability::RateLimited);
        assert_eq!(failed(&second.join().unwrap()), Unavailability::RateLimited);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let next = cache.read(renewed, || {
            panic!("effective token result must already be cached")
        });
        assert_eq!(failed(&next), Unavailability::RateLimited);
    }

    #[test]
    fn cancellation_and_forgetting_reject_late_results_without_poisoning_worker_slot() {
        let cache = Cache::default();
        let initial = key();
        let control = Arc::new(crate::scan::Control::default());
        let cancel = control.clone();
        let result = crate::scan::run(
            control,
            0,
            |_| {},
            || {
                cache.read(initial.clone(), || {
                    cancel.cancel();
                    (initial.clone(), HistoryRead::NotConfigured)
                })
            },
        );
        assert!(result.is_err());
        assert!(cache.state.lock().unwrap().entries.is_empty());
        let result = cache.read(initial.clone(), || {
            cache.forget(&initial.account);
            (initial.clone(), HistoryRead::NotConfigured)
        });
        assert_eq!(failed(&result), Unavailability::Loading);
        assert!(cache.state.lock().unwrap().entries.is_empty());
        let result = cache.read(initial.clone(), || panic!("synthetic reader failure"));
        assert_eq!(failed(&result), Unavailability::ServerError);
        assert!(!cache.state.lock().unwrap().running);
    }
}
