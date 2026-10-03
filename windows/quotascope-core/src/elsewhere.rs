//! Persist a cycle's external-use evidence, without claiming to detect
//! simultaneous work on two computers. Only live account-wide readings count.
use crate::ledger::UsageLedger;
use crate::model::{Provider, ProviderUsage, State, UsageWindow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Point {
    fraction: f64,
    at: i64,
    reset: i64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Watch {
    #[serde(default)]
    points: HashMap<String, Point>,
    #[serde(default)]
    cycles: HashMap<String, i64>,
}

impl Watch {
    pub fn load() -> Self {
        std::fs::read(crate::data_dir().join("used-elsewhere.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let directory = crate::data_dir();
        if std::fs::create_dir_all(&directory).is_err() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(self) else {
            return;
        };
        let temporary = directory.join("used-elsewhere.json.tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, directory.join("used-elsewhere.json"));
        }
    }

    pub fn observe(&mut self, reading: &ProviderUsage, ledger: &UsageLedger, now: i64) {
        self.points.retain(|_, p| p.reset > now);
        self.cycles.retain(|_, reset| *reset > now);
        if !matches!(
            reading.account.provider,
            Provider::ClaudeCode | Provider::Codex
        ) || !matches!(reading.state, State::Live)
            || ledger.has_partial_records
        {
            return;
        }
        let Some(at) = reading.observed_at.filter(|at| *at <= now) else {
            return;
        };
        for window in reading.windows.iter().filter(|w| w.scope.is_none()) {
            let Some(reset) = window.resets_at.filter(|reset| *reset > now) else {
                continue;
            };
            if !window.used_fraction.is_finite() {
                continue;
            }
            let key = format!("{}|{}", reading.account.id(), window.id);
            if let Some(previous) = self.points.get(&key) {
                if at.saturating_sub(previous.at) < 120_000 {
                    continue;
                }
                if reset.abs_diff(previous.reset) < 120_000 {
                    let start = previous.at.saturating_sub(900_000);
                    if Self::spent_elsewhere(
                        window.used_fraction - previous.fraction,
                        ledger.cost_between(start, at),
                        ledger.tokens_between(start, at),
                    ) {
                        self.cycles.insert(key.clone(), reset);
                    }
                }
            }
            self.points.insert(
                key,
                Point {
                    fraction: window.used_fraction,
                    at,
                    reset,
                },
            );
        }
    }

    pub fn spent_elsewhere(rise: f64, cost: f64, tokens: f64) -> bool {
        rise.is_finite()
            && cost.is_finite()
            && tokens.is_finite()
            && rise >= 0.02 - 1e-9
            && (0.0..0.05).contains(&cost)
            && (0.0..1000.0).contains(&tokens)
    }

    pub fn marked(&self, reading: &ProviderUsage, window: &UsageWindow) -> bool {
        window.resets_at.is_some_and(|reset| {
            self.cycles
                .get(&format!("{}|{}", reading.account.id(), window.id))
                .is_some_and(|saved| saved.abs_diff(reset) < 120_000)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountKey, Kind};

    #[test]
    fn unpriced_local_tokens_do_not_count_as_external_use() {
        assert!(Watch::spent_elsewhere(0.02, 0.0, 0.0));
        assert!(!Watch::spent_elsewhere(0.02, 0.0, 1000.0));
        assert!(!Watch::spent_elsewhere(0.01, 0.0, 0.0));
        assert!(!Watch::spent_elsewhere(0.03, f64::NAN, 0.0));
    }

    #[test]
    fn evidence_survives_restart_but_does_not_cross_account_or_cycle() {
        let mut reading = ProviderUsage::live_now(
            AccountKey::primary(Provider::Codex),
            vec![UsageWindow::new(
                "five",
                Kind::FiveHour,
                None,
                0.1,
                18_000,
                Some(10_000_000),
            )],
        );
        let mut watch = Watch::default();
        reading.observed_at = Some(1_000_000);
        watch.observe(&reading, &UsageLedger::empty(), 1_000_000);
        reading.windows[0].used_fraction = 0.13;
        reading.observed_at = Some(1_001_000);
        watch.observe(&reading, &UsageLedger::empty(), 1_001_000);
        assert!(!watch.marked(&reading, &reading.windows[0]));
        reading.observed_at = Some(1_120_000);
        watch.observe(&reading, &UsageLedger::empty(), 1_120_000);
        assert!(watch.marked(&reading, &reading.windows[0]));
        let restored: Watch = serde_json::from_slice(&serde_json::to_vec(&watch).unwrap()).unwrap();
        assert!(restored.marked(&reading, &reading.windows[0]));
        reading.account = AccountKey::primary(Provider::ClaudeCode);
        assert!(!restored.marked(&reading, &reading.windows[0]));
        reading.account = AccountKey::primary(Provider::Codex);
        reading.windows[0].resets_at = Some(30_000_000);
        assert!(!restored.marked(&reading, &reading.windows[0]));
    }
}
