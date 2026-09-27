//! Project-free instruction selection shared by the CLI and binary assertions.

pub(crate) fn render(command: &[&str]) -> Option<String> {
    Some(match command {
        ["help-instructions"] => baley::help::instructions::markdown(),
        ["spike-instructions"] => baley::spike::instructions::markdown().to_owned(),
        ["debug-instructions"] => baley::debug::instructions::markdown().to_owned(),
        ["undo-instructions"] => baley::undo::instructions::markdown().to_owned(),
        ["land-instructions"] => baley::landing::instructions::markdown().to_owned(),
        ["milestone-instructions"] => baley::milestone::instructions::markdown().to_owned(),
        ["suggest-instructions"] => baley::suggest::instructions::markdown().to_owned(),
        ["why-instructions"] => baley::why::instructions::markdown().to_owned(),
        ["progress-instructions"] => baley::progress::instructions::markdown().to_owned(),
        ["capture-instructions"] => baley::capture::instructions::markdown().to_owned(),
        ["context-instructions"] => baley::context::instructions::markdown(),
        ["plan-instructions"] => baley::plan::instructions::markdown(),
        ["read-instructions"] => baley::read::instructions::markdown(),
        ["task-instructions"] => baley::task::instructions::markdown(),
        ["executor-instructions"] => baley::execution::instructions::contract_markdown(),
        ["executor-instructions", "--frontdoor"] => {
            baley::execution::instructions::frontdoor_markdown()
        }
        ["verifier-instructions"] => baley::verification::instructions::contract_markdown(),
        ["verifier-instructions", "--frontdoor"] => {
            baley::verification::instructions::frontdoor_markdown()
        }
        ["review-instructions"] => {
            baley::review::instructions::frontdoor_markdown(baley::review::selection::CANONICAL)?
        }
        ["review-instructions", "--alias", alias] => {
            baley::review::instructions::frontdoor_markdown(alias)?
        }
        ["audit-instructions"] => {
            baley::verification::instructions::audit_frontdoor_markdown(false)
        }
        ["audit-instructions", "--coverage"] => {
            baley::verification::instructions::audit_frontdoor_markdown(true)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn rendered_command_identity_selects_the_requested_surface() {
        for (command, name) in [
            (&["help-instructions"][..], "bal-help"),
            (&["executor-instructions"][..], "bal-executor-contract"),
            (&["executor-instructions", "--frontdoor"][..], "bal-execute"),
            (&["verifier-instructions"][..], "bal-verifier-contract"),
            (&["verifier-instructions", "--frontdoor"][..], "bal-verify"),
            (&["review-instructions"][..], "bal-review"),
            (
                &["review-instructions", "--alias", "bal-decision-review"][..],
                "bal-decision-review",
            ),
            (
                &["review-instructions", "--alias", "bal-minimalism-review"][..],
                "bal-minimalism-review",
            ),
            (
                &["review-instructions", "--alias", "bal-plan-review"][..],
                "bal-plan-review",
            ),
            (&["audit-instructions"][..], "bal-audit"),
            (&["audit-instructions", "--coverage"][..], "bal-coverage"),
            (&["task-instructions"][..], "bal-task"),
        ] {
            let text = super::render(command).expect("instruction command");
            assert!(
                text.starts_with(&format!("---\nname: {name}\n")),
                "{command:?}"
            );
        }
        assert!(super::render(&["review-instructions", "--alias", "bal-help"]).is_none());
        assert!(super::render(&["executor-instructions", "--coverage"]).is_none());
    }
}
