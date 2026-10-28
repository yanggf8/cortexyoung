//! Shared classification of network failures.

pub(crate) const TIMEOUT_PREFIX: &str = "network timeout after ";

/// Prefix of a direct-remote failure that is the network's, not the store's: blocked, refused,
/// unreachable or busy after retries.
pub(crate) const REMOTE_PREFIX: &str = "remote unavailable: ";

/// Whether `message` is a network failure: a timeout, or a remote that could not serve.
pub(crate) fn network_failure(message: &str) -> bool {
    message.starts_with(TIMEOUT_PREFIX) || message.starts_with(REMOTE_PREFIX)
}
