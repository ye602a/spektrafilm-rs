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

/// Submission granularity for hosts that share the GPU with interactive work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilmSubmissionPolicy {
    /// Encode the complete physical chain and its consumer into one submission.
    #[default]
    WholeChain,
    /// When a cancellation signal is supplied, finish the negative before
    /// preparing and submitting paper/scan output. Inputs above 2048² pixels
    /// also yield at the retained optical and DIR boundaries. Each completed
    /// stage checks cancellation before preparing further work. Once the
    /// complete negative and its immutable snapshot finish on the GPU, that
    /// snapshot may be retained even if later output is cancelled or fails.
    /// Incomplete density and output frames are never published. No extra
    /// stages are submitted without a cancellation signal, or when a completed
    /// negative is reused.
    CooperativeStages,
}

/// Options belong to one invocation, so concurrent sources cannot overwrite
/// each other's identity or cancellation state in a shared backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct FilmRenderOptions<'a> {
    /// None bypasses negative-density caching. Work-allocation reuse is separate.
    pub source: Option<SourceIdentity>,
    /// Cooperative cancellation at preparation/submission/completion boundaries.
    /// Already submitted GPU work is allowed to finish before resources release.
    /// A cancelled call returns no output. WholeChain publishes no density on
    /// cancellation; CooperativeStages may retain an already completed negative.
    pub cancellation: Option<&'a AtomicBool>,
    /// Whole-chain submission is the default; interactive hosts may preserve
    /// their existing cooperative stage policy without slowing ordinary calls.
    pub submission_policy: FilmSubmissionPolicy,
    /// Give this invocation private work buffers and bypass the retained photo
    /// density snapshot, including cache statistics and publication tickets.
    /// Use for latitude/colour probes that share the photo device but must not
    /// disturb its retained image resources. Compiled pipelines still share.
    /// This is request-local and does not change other concurrent invocations.
    pub isolate_photo_caches: bool,
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
