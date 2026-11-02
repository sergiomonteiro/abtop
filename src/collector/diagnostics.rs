//! Collection metadata only: never include database values, transcripts, or commands.
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct CollectorDiagnostics {
    pub agent: &'static str,
    /// Symbolic location; environment overrides and private paths are not expanded.
    pub source: &'static str,
    pub schema: Option<&'static str>,
    pub state: &'static str,
    pub processes: usize,
    pub candidates: usize,
    pub accepted: usize,
    pub skipped: BTreeMap<&'static str, usize>,
    pub errors: Vec<&'static str>,
}

impl CollectorDiagnostics {
    pub fn new(agent: &'static str, source: &'static str) -> Self {
        Self {
            agent,
            source,
            schema: None,
            state: "empty",
            processes: 0,
            candidates: 0,
            accepted: 0,
            skipped: BTreeMap::new(),
            errors: Vec::new(),
        }
    }

    pub fn skip(&mut self, reason: &'static str) {
        *self.skipped.entry(reason).or_default() += 1;
    }

    pub fn fail(&mut self, reason: &'static str) {
        self.state = "error";
        if !self.errors.contains(&reason) {
            self.errors.push(reason);
        }
    }
}
