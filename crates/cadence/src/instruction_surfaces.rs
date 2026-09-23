//! Project-free instruction selection shared by the CLI and binary assertions.

pub(crate) fn render(command: &[&str]) -> Option<String> {
    Some(match command {
        ["help-instructions"] => cadence::help::instructions::markdown(),
        ["spike-instructions"] => cadence::spike::instructions::markdown(),
        ["debug-instructions"] => cadence::debug::instructions::markdown(),
        ["undo-instructions"] => cadence::undo::instructions::markdown(),
        ["land-instructions"] => cadence::landing::instructions::markdown(),
        ["milestone-instructions"] => cadence::milestone::instructions::markdown(),
        ["suggest-instructions"] => cadence::suggest::instructions::markdown(),
        ["why-instructions"] => cadence::why::instructions::markdown(),
        ["progress-instructions"] => cadence::progress::instructions::markdown(),
        ["capture-instructions"] => cadence::capture::instructions::markdown(),
        ["context-instructions"] => cadence::context::instructions::markdown(),
        ["plan-instructions"] => cadence::plan::instructions::markdown(),
        ["read-instructions"] => cadence::read::instructions::markdown(),
        ["task-instructions"] => cadence::task::instructions::markdown(),
        ["executor-instructions"] => cadence::execution::instructions::contract_markdown(),
        ["executor-instructions", "--frontdoor"] => cadence::execution::instructions::frontdoor_markdown(),
        ["verifier-instructions"] => cadence::verification::instructions::contract_markdown(),
        ["verifier-instructions", "--frontdoor"] => cadence::verification::instructions::frontdoor_markdown(),
        ["review-instructions"] => cadence::review::instructions::frontdoor_markdown(cadence::review::selection::CANONICAL)?,
        ["review-instructions", "--alias", alias] => cadence::review::instructions::frontdoor_markdown(alias)?,
        ["audit-instructions"] => cadence::verification::instructions::audit_frontdoor_markdown(false),
        ["audit-instructions", "--coverage"] => cadence::verification::instructions::audit_frontdoor_markdown(true),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn rendered_command_identity_selects_the_requested_surface() {
        for (command, name) in [
            (&["help-instructions"][..], "cad-help"),
            (&["executor-instructions"][..], "cad-executor-contract"),
            (&["executor-instructions", "--frontdoor"][..], "cad-execute"),
            (&["verifier-instructions"][..], "cad-verifier-contract"),
            (&["verifier-instructions", "--frontdoor"][..], "cad-verify"),
            (&["review-instructions"][..], "cad-review"),
            (&["review-instructions", "--alias", "cad-decision-review"][..], "cad-decision-review"),
            (&["review-instructions", "--alias", "cad-minimalism-review"][..], "cad-minimalism-review"),
            (&["review-instructions", "--alias", "cad-plan-review"][..], "cad-plan-review"),
            (&["audit-instructions"][..], "cad-audit"),
            (&["audit-instructions", "--coverage"][..], "cad-coverage"),
            (&["task-instructions"][..], "cad-task"),
        ] {
            let text = super::render(command).expect("instruction command");
            assert!(text.starts_with(&format!("---\nname: {name}\n")), "{command:?}");
        }
        assert!(super::render(&["review-instructions", "--alias", "cad-help"]).is_none());
        assert!(super::render(&["executor-instructions", "--coverage"]).is_none());
    }
}
