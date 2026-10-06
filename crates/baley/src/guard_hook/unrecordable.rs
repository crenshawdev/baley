//! Why a guard decision cannot be recorded (design 0010, GRD-R9): no
//! identity to record it by, or a store that could not record it. Each is
//! judged from plain values into the loud line the hook prints, and no
//! identity is ever invented in place of a missing one.

use crate::hook_input::Envelope;
use crate::mcp::context::DirectoryFault;
use baley_core::guard::AuditPrecondition;
use baley_core::policy::Host;
use baley_store::{HookCaller, StoreError};

/// The hook caller a call is recorded with, built from its envelope and
/// `CLAUDE_PROJECT_DIR` as given, or the line saying why the decision cannot
/// be recorded: no `tool_use_id`, a project directory that is not UTF-8, or
/// a cwd, project directory or session the caller refuses. A call with no
/// session is recorded without one. One the caller refuses is not, since
/// recording it without would key its redelivery under another session.
pub(super) fn caller(
    envelope: &Envelope,
    project_directory: &Result<Option<String>, DirectoryFault>,
) -> Result<HookCaller, String> {
    let Some(call) = &envelope.tool_use_id else {
        return Err(not_recorded("the call carries no tool_use_id"));
    };
    let directory = project_directory.as_ref().map_err(|fault| {
        not_recorded(&format!(
            "CLAUDE_PROJECT_DIR cannot be recorded as given ({fault})"
        ))
    })?;
    let refused =
        |error| not_recorded(&format!("the call's identity cannot be recorded ({error})"));
    let mut caller =
        HookCaller::new(Host::ClaudeCode.name(), &envelope.cwd, call).map_err(refused)?;
    if let Some(directory) = directory {
        caller = caller.with_project_directory(directory).map_err(refused)?;
    }
    if let Some(session) = &envelope.session_id {
        caller = caller.with_host_session(session).map_err(refused)?;
    }
    Ok(caller)
}

/// The precondition and the loud line for a store that could not record:
/// always unrecordable, with the cause named. The guard never rebuilds views
/// or waits for the maintenance lock, so views that need a rebuild name the
/// owner's command for it.
pub(super) fn store_failure(error: &StoreError) -> (AuditPrecondition, String) {
    let line = match error {
        StoreError::Busy => not_recorded("the ledger stayed busy past the guard's storage time"),
        StoreError::NeedsRebuild { project } => format!(
            "{}. Run baley rebuild {} to bring its views current.",
            not_recorded(&format!("the {} project's views need a rebuild", project.0)),
            project.0
        ),
        error => not_recorded(&format!("the ledger could not record it ({error})")),
    };
    (AuditPrecondition::Unrecordable, line)
}

/// The loud line for a call whose id was answered before for another input,
/// project directory or cwd.
pub(super) fn clash() -> String {
    not_recorded("this call id was answered before for another input, project directory or cwd")
}

/// The loud line for a decision that is not recorded.
pub(super) fn not_recorded(cause: &str) -> String {
    format!("Baley guard: {cause}, so this decision is not recorded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use baley_core::guard::{Answer, record_answer};
    use baley_store::ProjectId;

    fn envelope() -> Envelope {
        Envelope {
            cwd: "/q".into(),
            session_id: Some("s".into()),
            tool_use_id: Some("t1".into()),
        }
    }

    fn given(directory: &str) -> Result<Option<String>, DirectoryFault> {
        Ok(Some(directory.into()))
    }

    #[test]
    fn a_failure_pass_denied_or_an_identity_invented_for_a_missing_tool_use_id_is_caught() {
        let anonymous = Envelope {
            tool_use_id: None,
            ..envelope()
        };

        let judged = caller(&anonymous, &given("/p"));

        let Err(line) = judged else {
            panic!("unrecordable: {judged:?}");
        };
        assert!(line.contains("tool_use_id"), "{line}");
        let unrecordable = AuditPrecondition::Unrecordable;
        let loud = Answer::PassOnFailure("git could not read the branch".into());
        assert_eq!(record_answer(loud.clone(), unrecordable), loud);
        assert!(matches!(
            record_answer(Answer::Ask("approve?".into()), unrecordable),
            Answer::Deny(_)
        ));
    }

    #[test]
    fn a_relative_cwd_recorded_is_caught() {
        let relative = Envelope {
            cwd: "q".into(),
            ..envelope()
        };
        assert!(caller(&relative, &given("/p")).is_err());
    }

    #[test]
    fn a_call_with_no_session_left_unrecordable_is_caught() {
        let sessionless = Envelope {
            session_id: None,
            ..envelope()
        };
        let recorded = caller(&sessionless, &given("/p")).expect("recordable");
        assert_eq!(recorded.host_session(), None);
    }

    #[test]
    fn a_project_directory_recorded_other_than_as_given_is_caught() {
        let recorded = caller(&envelope(), &given("/p/../p/")).expect("recordable");
        assert_eq!(recorded.project_directory(), Some("/p/../p/"));
        assert_eq!(recorded.working_directory(), "/q");
        assert_eq!(recorded.call().text(), "t1");
        assert_eq!(recorded.host_session(), Some("s"));

        let not_text = Err(DirectoryFault::NotUtf8);
        assert!(caller(&envelope(), &not_text).is_err());
    }

    #[test]
    fn the_cwd_recorded_as_the_project_directory_when_none_was_given_is_caught() {
        let recorded = caller(&envelope(), &Ok(None)).expect("recordable");
        assert_eq!(recorded.project_directory(), None);
        assert_eq!(recorded.working_directory(), "/q");
    }

    #[test]
    fn a_busy_or_unbuilt_store_read_as_recorded_or_naming_no_command_is_caught() {
        let (busy, line) = store_failure(&StoreError::Busy);
        assert_eq!(busy, AuditPrecondition::Unrecordable);
        assert!(line.contains("busy"), "{line}");

        let needs = StoreError::NeedsRebuild {
            project: ProjectId("user".into()),
        };
        let (unbuilt, line) = store_failure(&needs);
        assert_eq!(unbuilt, AuditPrecondition::Unrecordable);
        assert!(line.contains("baley rebuild user"), "{line}");
    }
}
