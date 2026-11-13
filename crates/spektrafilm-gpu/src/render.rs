//! Per-call rendering identity and cooperative cancellation.
use std::sync::atomic::{AtomicBool, Ordering};

use crate::output::OutputError;

/// Host-owned identity of immutable input pixels.
///
/// Change the token whenever the input pixels change, including RAW processing
/// or edits before this engine. Reuse it across paper/scanner/display edits.
/// The engine adds every dependency of its own pre-print stages to this token;
/// hosts must not duplicate film-parameter invalidation logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceIdentity(pub u128);

/// Options belong to one invocation, so concurrent sources cannot overwrite
/// each other's identity or cancellation state in a shared backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct FilmRenderOptions<'a> {
    /// None bypasses negative-density caching. Work-allocation reuse is separate.
    pub source: Option<SourceIdentity>,
    /// Cooperative cancellation at preparation/submission/completion boundaries.
    /// Already submitted GPU work is allowed to finish before resources release.
    /// A cancelled call returns no output and publishes no density snapshot.
    pub cancellation: Option<&'a AtomicBool>,
}

impl FilmRenderOptions<'_> {
    /// Check the host's cancellation flag without submitting any GPU work.
    pub fn check_cancelled(self) -> Result<(), OutputError> {
        if self
            .cancellation
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            Err(OutputError::Cancelled)
        } else {
            Ok(())
        }
    }
}
