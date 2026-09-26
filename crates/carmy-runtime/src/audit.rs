//! Who ran what, when, and how it ended. Records never carry arguments or outputs:
//! they may hold secrets, and an audit trail must be safe to keep and to show.
use carmy_core::{Effect, ExecutionStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Mutex};

/// One finished execution, including replays and requests rejected by a policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub execution_id: String,
    pub request_id: Option<String>,
    pub principal: Option<String>,
    pub session: Option<String>,
    pub tool: String,
    /// Absent when the tool is unknown.
    pub effect: Option<Effect>,
    pub status: ExecutionStatus,
    pub error_code: Option<String>,
    pub duration_ms: u64,
    pub started_at: DateTime<Utc>,
    /// The result came from the idempotency store; the tool did not run.
    pub replayed: bool,
}

/// Receives a record after every execution. `record` runs on the execution's task, so
/// it must return at once: durable sinks hand the record to a channel or a task.
pub trait ExecutionSink: Send + Sync {
    fn record(&self, record: ExecutionRecord);
}

/// The last `capacity` records, in memory. Fine for development, the console and tests.
pub struct InMemoryAudit {
    records: Mutex<VecDeque<ExecutionRecord>>,
    capacity: usize,
}

impl InMemoryAudit {
    pub fn new(capacity: usize) -> Self {
        Self {
            records: Mutex::new(VecDeque::with_capacity(capacity.min(1024))),
            capacity: capacity.max(1),
        }
    }
    /// Up to `limit` records, newest first.
    pub fn recent(&self, limit: usize) -> Vec<ExecutionRecord> {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        records.iter().rev().take(limit).cloned().collect()
    }
    pub fn len(&self) -> usize {
        self.records.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for InMemoryAudit {
    fn default() -> Self {
        Self::new(1_000)
    }
}

impl ExecutionSink for InMemoryAudit {
    fn record(&self, record: ExecutionRecord) {
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if records.len() == self.capacity {
            records.pop_front();
        }
        records.push_back(record);
    }
}
