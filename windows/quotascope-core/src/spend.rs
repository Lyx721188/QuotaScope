//! Calendar-scoped analysis of measured local agent records. No quota or
//! subscription estimate is substituted for a missing transcript counter.
use crate::ledger::{TokenCost, TokenTally, UsageLedger};
use crate::model::Provider;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Span {
    Today,
    #[default]
    Week,
    Month,
    Quarter,
    All,
}
impl Span {
    pub const ALL: [Self; 5] = [
        Self::Today,
        Self::Week,
        Self::Month,
        Self::Quarter,
        Self::All,
    ];
    pub fn contains(self, date: NaiveDate, today: NaiveDate) -> bool {
        let age = today.signed_duration_since(date).num_days();
        age >= 0
            && match self {
                Self::Today => age < 1,
                Self::Week => age < 7,
                Self::Month => age < 30,
                Self::Quarter => age < 90,
                Self::All => true,
            }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub id: String,
    pub title: String,
    pub location: String,
    pub present: bool,
    pub ledger: UsageLedger,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub sources: Vec<Source>,
}
impl Snapshot {
    /// Called by a worker only, and only after the local-reading opt-in.
    pub fn read_native() -> Self {
        if !crate::settings::with(|s| s.reads_token_spend) {
            return Self::default();
        }
        macro_rules! read_source {
            ($name:expr, $read:expr) => {{
                if !crate::scan::checkpoint() {
                    return Self::default();
                }
                crate::scan::source_started($name);
                let ledger = $read;
                if !crate::scan::checkpoint() {
                    return Self::default();
                }
                crate::scan::source_finished();
                ledger
            }};
        }
        let mut sources = Vec::new();
        for (id, title, client) in [
            ("opencode", "OpenCode", "opencode"),
            ("kilo", "Kilo CLI", "kilo"),
        ] {
            let path = crate::opencode_store::path(client);
            sources.push(Source {
                id: id.into(),
                title: title.into(),
                location: path.display().to_string(),
                present: path.is_file(),
                ledger: read_source!(title, crate::opencode_store::read(&path)),
            });
        }
        for p in [Provider::ClaudeCode, Provider::Codex] {
            let path = crate::ledger::transcript_root(p);
            let ledger = read_source!(p.display_name(), crate::ledger::ledger(p));
            sources.push(Source {
                id: p.raw().into(),
                title: p.display_name().into(),
                location: path
                    .as_ref()
                    .map(|v| v.display().to_string())
                    .unwrap_or_default(),
                present: path.as_ref().is_some_and(|v| v.is_dir()),
                ledger,
            });
        }
        let path = crate::ledger::qwen_root();
        sources.push(Source {
            id: "qwen".into(),
            title: "Qwen Code".into(),
            location: path.display().to_string(),
            present: path.is_dir(),
            ledger: read_source!("Qwen Code", crate::ledger::qwen_ledger()),
        });
        let path = crate::model::home_path(".gemini/tmp");
        sources.push(Source {
            id: "gemini".into(),
            title: "Gemini CLI".into(),
            location: path.display().to_string(),
            present: path.is_dir(),
            ledger: read_source!("Gemini CLI", crate::ledger::gemini_ledger()),
        });
        for client in [crate::ledger::PiClient::Pi, crate::ledger::PiClient::Omp] {
            let path = client.root();
            let path = path.first().cloned().unwrap_or_default();
            sources.push(Source {
                id: client.id().into(),
                title: match client {
                    crate::ledger::PiClient::Pi => "Pi",
                    crate::ledger::PiClient::Omp => "Oh My Pi",
                    crate::ledger::PiClient::Senpi => "OmO Native",
                    crate::ledger::PiClient::Kimchi => "Kimchi",
                }
                .into(),
                location: path.display().to_string(),
                present: path.is_dir(),
                ledger: read_source!(
                    if client == crate::ledger::PiClient::Pi {
                        "Pi"
                    } else {
                        "Oh My Pi"
                    },
                    crate::ledger::pi_ledger(client)
                ),
            });
        }
        sources.push(Source {
            id: "senpi".into(),
            title: "OmO Native".into(),
            location: crate::ledger::PiClient::Senpi
                .root()
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            present: crate::ledger::PiClient::Senpi
                .root()
                .iter()
                .any(|path| path.is_dir()),
            ledger: read_source!(
                "OmO Native",
                crate::ledger::pi_ledger(crate::ledger::PiClient::Senpi)
            ),
        });
        sources.push(Source {
            id: "kimchi".into(),
            title: "Kimchi".into(),
            location: crate::ledger::PiClient::Kimchi
                .root()
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            present: crate::ledger::PiClient::Kimchi
                .root()
                .iter()
                .any(|path| path.is_dir()),
            ledger: read_source!(
                "Kimchi",
                crate::ledger::pi_ledger(crate::ledger::PiClient::Kimchi)
            ),
        });
        for agent in [
            crate::ledger::GenericAgent::Amp,
            crate::ledger::GenericAgent::Droid,
        ] {
            let path = agent.root();
            sources.push(Source {
                id: agent.id().into(),
                title: match agent {
                    crate::ledger::GenericAgent::Amp => "Amp",
                    crate::ledger::GenericAgent::Droid => "Droid",
                }
                .into(),
                location: path.display().to_string(),
                present: path.is_dir(),
                ledger: read_source!(
                    if agent == crate::ledger::GenericAgent::Amp {
                        "Amp"
                    } else {
                        "Droid"
                    },
                    crate::ledger::generic_agent_ledger(agent)
                ),
            });
        }
        let path = crate::model::home_path(".prime/agent");
        sources.push(Source {
            id: "prime-agent".into(),
            title: "Prime Agent".into(),
            location: path.display().to_string(),
            present: path.is_dir(),
            ledger: read_source!("Prime Agent", crate::ledger::prime_agent_ledger()),
        });
        let path = crate::model::home_path(".openclaw/agents");
        sources.push(Source {
            id: "openclaw".into(),
            title: "OpenClaw".into(),
            location: path.display().to_string(),
            present: path.is_dir(),
            ledger: read_source!("OpenClaw", crate::ledger::openclaw_ledger()),
        });
        for (id, title, root, read) in [
            (
                "mux",
                "Mux",
                ".mux/sessions",
                crate::ledger::mux_ledger as fn() -> UsageLedger,
            ),
            (
                "junie",
                "Junie",
                ".junie/sessions",
                crate::ledger::junie_ledger as fn() -> UsageLedger,
            ),
            (
                "augment",
                "Augment",
                ".augment/sessions",
                crate::ledger::augment_ledger as fn() -> UsageLedger,
            ),
            (
                "jcode",
                "JCode",
                ".jcode/sessions",
                crate::ledger::jcode_ledger as fn() -> UsageLedger,
            ),
            (
                "gjc",
                "Gajae Code",
                ".gjc/agent/sessions",
                crate::ledger::gjc_ledger as fn() -> UsageLedger,
            ),
            (
                "codebuff",
                "Codebuff",
                ".config/manicode",
                crate::ledger::codebuff_ledger as fn() -> UsageLedger,
            ),
            (
                "fx",
                "FX",
                ".fx/sessions",
                crate::ledger::fx_ledger as fn() -> UsageLedger,
            ),
            (
                "reasonix",
                "Reasonix",
                ".reasonix/stats",
                crate::ledger::reasonix_ledger as fn() -> UsageLedger,
            ),
            (
                "lmstudio",
                "LM Studio",
                ".lmstudio/server-logs",
                crate::ledger::lmstudio_ledger as fn() -> UsageLedger,
            ),
        ] {
            let path = crate::model::home_path(root);
            sources.push(Source {
                id: id.into(),
                title: title.into(),
                location: path.display().to_string(),
                present: path.is_dir(),
                ledger: read_source!(title, read()),
            });
        }
        for (id, title, relative) in crate::additional_spend::CATALOG {
            let paths = crate::additional_spend::roots(id, relative);
            sources.push(Source {
                id: (*id).into(),
                title: (*title).into(),
                location: paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
                present: paths.iter().any(|p| p.exists()),
                ledger: read_source!(*title, crate::additional_spend::read(id, &paths)),
            });
        }
        let path = crate::ledger::transcript_root(Provider::Antigravity);
        sources.push(Source {
            id: "antigravity-ide".into(),
            title: "Antigravity IDE".into(),
            location: path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            present: path.as_ref().is_some_and(|p| p.exists()),
            ledger: read_source!(
                "Antigravity IDE",
                crate::ledger::ledger(Provider::Antigravity)
            ),
        });
        Self { sources }
    }

    pub fn read_controlled(
        control: std::sync::Arc<crate::scan::Control>,
        refresh: bool,
        progress: impl Fn(crate::scan::Progress) + 'static,
    ) -> Result<Self, crate::scan::Cancelled> {
        crate::scan::run(control, 54, progress, || {
            if refresh {
                crate::ledger::invalidate_memory();
            }
            Self::read_native()
        })
    }

    /// Empty hours remain visible; model filters use recorded splits only.
    pub fn hourly(
        &self,
        span: Span,
        agent: Option<&str>,
        model: Option<&str>,
        today: NaiveDate,
    ) -> [i64; 24] {
        use chrono::Timelike;
        let mut hours = [0_i64; 24];
        for source in self
            .sources
            .iter()
            .filter(|s| agent.is_none_or(|a| a == s.id))
        {
            if !crate::scan::checkpoint() {
                return hours;
            }
            for slot in &source.ledger.slots {
                if !crate::scan::checkpoint() {
                    return hours;
                }
                let Some(at) = chrono::DateTime::from_timestamp_millis(slot.start_ms)
                    .map(|t| t.with_timezone(&chrono::Local))
                else {
                    continue;
                };
                if span.contains(at.date_naive(), today) {
                    let tokens = model
                        .map(|m| {
                            slot.models.get(m).map(TokenTally::total).unwrap_or(0)
                                + slot.unclassified_models.get(m).copied().unwrap_or(0)
                        })
                        .unwrap_or(slot.tokens);
                    let hour = at.hour() as usize;
                    hours[hour] = hours[hour].saturating_add(tokens);
                }
            }
        }
        hours
    }

    pub fn analyze(
        &self,
        span: Span,
        today: NaiveDate,
        source: Option<&str>,
        model: Option<&str>,
        group: Group,
        sort: Sort,
        descending: bool,
    ) -> Analysis {
        let mut out = Analysis::default();
        let mut rows = BTreeMap::<String, Row>::new();
        let mut daily = BTreeMap::<NaiveDate, Measures>::new();
        for agent in self
            .sources
            .iter()
            .filter(|s| source.is_none_or(|id| s.id == id))
        {
            if !crate::scan::checkpoint() {
                return Analysis::default();
            }
            for day in agent
                .ledger
                .days
                .iter()
                .filter(|d| span.contains(d.date, today))
            {
                if !crate::scan::checkpoint() {
                    return Analysis::default();
                }
                let mut measured = Measures::default();
                // The source total stays authoritative, including unclassified
                // work that cannot honestly be attributed to a model.
                if model.is_none() {
                    measured = Measures {
                        tokens: day.tokens,
                        tally: day.tally,
                        cost: day.cost,
                        costs: day
                            .model_costs
                            .values()
                            .copied()
                            .fold(TokenCost::default(), |a, b| a + b),
                        unpriced: day.unpriced_tokens,
                        unclassified: (day.tokens - day.tally.total()).max(0),
                    };
                }
                for (id, tokens) in day
                    .models
                    .iter()
                    .filter(|(id, _)| model.is_none_or(|m| m == id.as_str()))
                {
                    if !crate::scan::checkpoint() {
                        return Analysis::default();
                    }
                    let tally = day.model_tallies.get(id).copied().unwrap_or_default();
                    let costs = day.model_costs.get(id).copied();
                    let values = Measures {
                        tokens: *tokens,
                        tally,
                        costs: costs.unwrap_or_default(),
                        cost: costs.map(|c| c.total()).unwrap_or_default(),
                        unpriced: if costs.is_none() {
                            *tokens
                        } else {
                            (*tokens - tally.total()).max(0)
                        },
                        unclassified: (*tokens - tally.total()).max(0),
                    };
                    if model.is_some() {
                        measured.add(values);
                    }
                    if group == Group::Models {
                        let name = agent
                            .ledger
                            .model_names
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| id.clone());
                        let row = rows.entry(id.clone()).or_insert_with(|| Row {
                            id: id.clone(),
                            title: name,
                            measures: Measures::default(),
                        });
                        row.measures.add(values);
                    }
                }
                if group == Group::Agents {
                    rows.entry(agent.id.clone())
                        .or_insert_with(|| Row {
                            id: agent.id.clone(),
                            title: agent.title.clone(),
                            measures: Measures::default(),
                        })
                        .measures
                        .add(measured);
                }
                out.total.add(measured);
                daily.entry(day.date).or_default().add(measured);
            }
        }
        out.days = daily
            .into_iter()
            .map(|(date, measures)| Day { date, measures })
            .collect();
        if group == Group::Days {
            rows = out
                .days
                .iter()
                .map(|d| {
                    let id = d.date.to_string();
                    (
                        id.clone(),
                        Row {
                            id: id.clone(),
                            title: id,
                            measures: d.measures,
                        },
                    )
                })
                .collect();
        }
        out.rows = rows.into_values().collect();
        out.sort(sort, descending);
        out
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Group {
    #[default]
    Agents,
    Models,
    Days,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    Name,
    #[default]
    Tokens,
    Cost,
    Input,
    Output,
    CacheRead,
    CacheWrite,
}
impl Sort {
    pub const ALL: [Self; 7] = [
        Self::Name,
        Self::Tokens,
        Self::Cost,
        Self::Input,
        Self::Output,
        Self::CacheRead,
        Self::CacheWrite,
    ];
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Measures {
    pub tokens: i64,
    pub tally: TokenTally,
    pub cost: f64,
    pub costs: TokenCost,
    pub unpriced: i64,
    pub unclassified: i64,
}
impl Measures {
    fn add(&mut self, other: Self) {
        self.tokens += other.tokens;
        self.tally += other.tally;
        self.cost += other.cost;
        self.costs += other.costs;
        self.unpriced += other.unpriced;
        self.unclassified += other.unclassified;
    }
    pub fn cache_hit_rate(self) -> Option<f64> {
        let input = self.tally.input + self.tally.cache_read + self.tally.cache_write;
        (input > 0 && self.unclassified == 0).then(|| self.tally.cache_read as f64 / input as f64)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub measures: Measures,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub measures: Measures,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Analysis {
    pub total: Measures,
    pub rows: Vec<Row>,
    pub days: Vec<Day>,
}
impl Analysis {
    pub fn page(&self, index: usize, size: usize) -> &[Row] {
        let from = index.saturating_mul(size).min(self.rows.len());
        &self.rows[from..from.saturating_add(size).min(self.rows.len())]
    }

    /// Reorders a complete aggregation without revisiting its sources.
    pub fn sort(&mut self, sort: Sort, descending: bool) {
        self.rows.sort_by(|a, b| {
            let ordering = match sort {
                Sort::Name => a.title.cmp(&b.title),
                Sort::Tokens => a.measures.tokens.cmp(&b.measures.tokens),
                Sort::Cost => a.measures.cost.total_cmp(&b.measures.cost),
                Sort::Input => a.measures.tally.input.cmp(&b.measures.tally.input),
                Sort::Output => a.measures.tally.output.cmp(&b.measures.tally.output),
                Sort::CacheRead => a
                    .measures
                    .tally
                    .cache_read
                    .cmp(&b.measures.tally.cache_read),
                Sort::CacheWrite => a
                    .measures
                    .tally
                    .cache_write
                    .cmp(&b.measures.tally.cache_write),
            };
            (if descending {
                ordering.reverse()
            } else {
                ordering
            })
            .then(a.id.cmp(&b.id))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn source(id: &str) -> Source {
        let mut buckets = BTreeMap::new();
        for (d, model, tally) in [
            (
                "2026-09-24 09:00",
                "known",
                TokenTally {
                    input: 10,
                    output: 20,
                    ..Default::default()
                },
            ),
            (
                "2026-10-01 09:00",
                "known",
                TokenTally {
                    input: 100,
                    cache_read: 400,
                    output: 50,
                    ..Default::default()
                },
            ),
            (
                "2026-10-01 10:00",
                "unknown",
                TokenTally {
                    input: 60,
                    ..Default::default()
                },
            ),
            (
                "2026-10-02 09:00",
                "known",
                TokenTally {
                    input: 900,
                    ..Default::default()
                },
            ),
        ] {
            buckets.insert(d.into(), BTreeMap::from([(model.into(), tally)]));
        }
        let prices = BTreeMap::from([(
            "known".into(),
            crate::model_prices::ModelPrice {
                input: 2.0,
                output: 10.0,
                cache_read: Some(0.2),
                cache_write: None,
                name: None,
            },
        )]);
        Source {
            id: id.into(),
            title: id.into(),
            location: String::new(),
            present: true,
            ledger: crate::ledger::priced(&buckets, &prices, None),
        }
    }
    #[test]
    fn calendar_scope_model_drilldown_and_price_gaps() {
        let s = Snapshot {
            sources: vec![source("a"), source("b")],
        };
        let a = s.analyze(
            Span::Week,
            date("2026-10-01"),
            Some("a"),
            None,
            Group::Models,
            Sort::Tokens,
            true,
        );
        assert_eq!(a.total.tokens, 610);
        assert_eq!(a.total.unpriced, 60);
        assert_eq!(a.rows[0].id, "known");
        assert!((a.total.cost - 0.00078).abs() < 1e-10);
        assert_eq!(a.total.costs.total(), a.total.cost);
        let m = s.analyze(
            Span::Today,
            date("2026-10-01"),
            None,
            Some("unknown"),
            Group::Agents,
            Sort::Cost,
            false,
        );
        assert_eq!(m.total.tokens, 120);
        assert_eq!(m.total.unpriced, 120);
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.page(usize::MAX, 20).len(), 0);
        assert_eq!(m.page(0, 1).len(), 1);
    }
    #[test]
    fn unidentified_work_is_never_assigned_to_a_known_model() {
        let mut s = source("a");
        let today = date("2026-10-01");
        let day = s.ledger.days.iter_mut().find(|d| d.date == today).unwrap();
        day.tokens += 7;
        day.unpriced_tokens += 7;
        let a = Snapshot { sources: vec![s] }.analyze(
            Span::Today,
            today,
            None,
            None,
            Group::Models,
            Sort::Name,
            false,
        );
        assert_eq!(a.total.tokens, 617);
        assert_eq!(a.total.unclassified, 7);
        assert!(a.total.cache_hit_rate().is_none());
        assert_eq!(a.rows.iter().map(|r| r.measures.tokens).sum::<i64>(), 610);
    }
}
