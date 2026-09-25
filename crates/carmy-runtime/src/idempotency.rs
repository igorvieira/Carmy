//! Atomic request reservation. Adapters must retain uncertain reservations.
use crate::error;
use carmy_core::*;
use std::{collections::BTreeMap, future::Future, pin::Pin};
use tokio::sync::Mutex;
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = AgentResult<T>> + Send + 'a>>;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdempotencyKey {
    pub principal: Option<String>,
    pub session: Option<String>,
    pub request_id: String,
}
#[derive(Debug, Clone)]
pub enum Reservation {
    Acquired,
    Replay(ExecutionResult),
    InProgress,
    Conflict,
}
/// `reserve` must atomically compare the fingerprint and reserve vacant identities.
/// `complete` must durably retain the result before returning success. Never evict
/// in-progress/uncertain entries: they may represent an already committed side effect.
pub trait IdempotencyStore: Send + Sync {
    fn reserve<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
    ) -> StoreFuture<'a, Reservation>;
    fn complete<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
        result: &'a ExecutionResult,
    ) -> StoreFuture<'a, ()>;
}
struct Entry {
    fingerprint: String,
    result: Option<ExecutionResult>,
}
/// Bounded, process-local store. No automatic eviction; exhaustion fails closed.
pub struct InMemoryIdempotencyStore {
    entries: Mutex<BTreeMap<IdempotencyKey, Entry>>,
    capacity: usize,
}
impl InMemoryIdempotencyStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            capacity,
        }
    }
}
impl Default for InMemoryIdempotencyStore {
    fn default() -> Self {
        Self::new(10_000)
    }
}
impl IdempotencyStore for InMemoryIdempotencyStore {
    fn reserve<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
    ) -> StoreFuture<'a, Reservation> {
        Box::pin(async move {
            let mut entries = self.entries.lock().await;
            if let Some(entry) = entries.get(key) {
                return Ok(if entry.fingerprint != fingerprint {
                    Reservation::Conflict
                } else if let Some(result) = &entry.result {
                    Reservation::Replay(result.clone())
                } else {
                    Reservation::InProgress
                });
            }
            if entries.len() >= self.capacity {
                return Err(error(
                    "IDEMPOTENCY_CAPACITY",
                    "Idempotency store is full",
                    ErrorCategory::Capacity,
                ));
            }
            entries.insert(
                key.clone(),
                Entry {
                    fingerprint: fingerprint.into(),
                    result: None,
                },
            );
            Ok(Reservation::Acquired)
        })
    }
    fn complete<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
        result: &'a ExecutionResult,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut entries = self.entries.lock().await;
            match entries.get_mut(key) {
                Some(entry) if entry.fingerprint == fingerprint && entry.result.is_none() => {
                    entry.result = Some(result.clone());
                    Ok(())
                }
                _ => Err(error(
                    "IDEMPOTENCY_CONFLICT",
                    "Reservation no longer matches",
                    ErrorCategory::Conflict,
                )),
            }
        })
    }
}
