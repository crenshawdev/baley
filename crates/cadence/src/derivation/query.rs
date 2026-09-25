use super::*;
use std::path::Path;

/// Validated derivation and its immutable evidence. Publication still requires
/// a final recheck, immediately before return or submission to the writer.
#[derive(Debug)]
pub struct PreparedLifecycle {
    capture: CapturedInputs,
    overlay: AcceptanceOverlay,
    answer: Lifecycle,
}

impl PreparedLifecycle {
    pub fn capture(&self) -> &CapturedInputs {
        &self.capture
    }
    /// The native acceptance authority this derivation was made with.
    pub fn overlay(&self) -> &AcceptanceOverlay {
        &self.overlay
    }
    pub fn answer(&self) -> &Lifecycle {
        &self.answer
    }
    /// The memo identity of every input, native authority included.
    pub fn input_key(&self) -> Result<String, DerivationError> {
        super::memo::input_key_with(&self.capture, &self.overlay)
    }
}

/// Constructible only through successful reobservation. This is an observation
/// checkpoint, with no guarantee against edits after that checkpoint.
#[derive(Debug)]
pub struct RecheckedLifecycle {
    prepared: PreparedLifecycle,
}

impl RecheckedLifecycle {
    pub fn capture(&self) -> &CapturedInputs {
        &self.prepared.capture
    }
    pub fn overlay(&self) -> &AcceptanceOverlay {
        &self.prepared.overlay
    }
    pub fn answer(&self) -> &Lifecycle {
        &self.prepared.answer
    }
}

pub fn prepare_query(
    selected: &Path,
    io: &mut (impl ArtifactIo + ?Sized),
) -> Result<PreparedLifecycle, DerivationError> {
    prepare(selected, io, false)
}

/// Only progress reports roadmap conflicts instead of refusing the read.
pub fn prepare_progress(selected: &Path, io: &mut (impl ArtifactIo + ?Sized)) -> Result<PreparedLifecycle, DerivationError> {
    prepare(selected, io, true)
}

fn prepare(
    selected: &Path,
    io: &mut (impl ArtifactIo + ?Sized),
    report_conflicts: bool,
) -> Result<PreparedLifecycle, DerivationError> {
    let capture = capture_inputs(selected, io)?;
    // Native acceptance is read from the store files beside the artifacts,
    // before any consistency check, so a natively completed phase agrees
    // with its checked box without SUMMARY.md or UAT.md (D-131).
    let overlay = io.acceptance(&capture.root)?;
    let answer = derive_with(&capture, &overlay)?;
    if !report_conflicts {
        check_consistency(validate_inputs(&capture)?, &answer)?;
    }
    Ok(PreparedLifecycle { capture, overlay, answer })
}

pub fn recheck_query(
    prepared: &PreparedLifecycle,
    io: &mut (impl ArtifactIo + ?Sized),
) -> Result<RecheckedLifecycle, DerivationError> {
    // Reuse the resolved address: a changed working directory cannot retarget
    // the second observation of a relative selection.
    let second = capture_inputs(&prepared.capture.root, io)?;
    validate_observation_failures(&second)?;
    if second != prepared.capture || io.acceptance(&prepared.capture.root)? != prepared.overlay {
        return Err(DerivationError::InputsChanged);
    }
    Ok(RecheckedLifecycle {
        prepared: PreparedLifecycle {
            capture: prepared.capture.clone(),
            overlay: prepared.overlay.clone(),
            answer: prepared.answer.clone(),
        },
    })
}

pub fn query(
    selected: &Path,
    io: &mut (impl ArtifactIo + ?Sized),
) -> Result<RecheckedLifecycle, DerivationError> {
    let prepared = prepare_query(selected, io)?;
    recheck_query(&prepared, io)
}
