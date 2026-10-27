//! Shared bound for Turso replica bootstrap and sync calls.

use std::future::Future;
use std::time::Duration;

pub(crate) const TIMEOUT_PREFIX: &str = "network timeout after ";
pub(crate) const POISONED: &str = "network disabled after a timeout; open the store again";

pub(crate) fn timed_out(message: &str) -> bool {
    message.starts_with(TIMEOUT_PREFIX)
}

/// Prefix of a direct-remote failure that is the network's, not the store's: blocked, refused,
/// unreachable or busy after retries.
pub(crate) const REMOTE_PREFIX: &str = "remote unavailable: ";

/// Whether `message` is a network failure: a timeout, or a remote that could not serve.
pub(crate) fn network_failure(message: &str) -> bool {
    timed_out(message) || message.starts_with(REMOTE_PREFIX)
}

/// Run one network future under `limit`. The turso client retries on its own schedule, and a
/// process that hangs in it has no other way to be reclaimed (there is no cron and the hook only
/// kicks), which is why the bound is owned here and not by the client. Whole seconds, truncated,
/// in the message. The timeout arms a timer inside the caller's runtime.
pub(crate) async fn with_timeout_async<T, E: std::fmt::Display>(
    limit: Duration,
    fut: impl Future<Output = Result<T, E>>,
) -> Result<T, String> {
    match tokio::time::timeout(limit, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("{TIMEOUT_PREFIX}{}s", limit.as_secs())),
    }
}

/// One store's network limit and the flag that outlives it, which is not reusable after a
/// timeout.
///
/// Reusing it would mean pushing on top of a sync engine whose in-flight future was dropped
/// mid-operation, and nothing here knows what that left half-done. Reopening instead is always
/// safe: every local write reached the replica before the network call, and the next push is
/// idempotent (measure events are keyed and `INSERT OR IGNORE`d; ledger entries and notes are
/// keyed by id and primary key).
#[derive(Debug, Clone, Copy)]
pub(crate) struct NetworkGuard {
    limit: Duration,
    poisoned: bool,
}

impl NetworkGuard {
    pub(crate) fn new(limit: Duration) -> Self {
        Self {
            limit,
            poisoned: false,
        }
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        if self.poisoned {
            Err(POISONED.to_string())
        } else {
            Ok(())
        }
    }

    pub(crate) fn observe<T>(&mut self, result: &Result<T, String>) {
        if matches!(result, Err(message) if timed_out(message)) {
            self.poisoned = true;
        }
    }

    pub(crate) async fn run<T, E: std::fmt::Display>(
        &mut self,
        fut: impl Future<Output = Result<T, E>>,
    ) -> Result<T, String> {
        self.check()?;
        let result = with_timeout_async(self.limit, fut).await;
        self.observe(&result);
        result
    }
}
