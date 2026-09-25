#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Diagnostics, Effective, reload::Input};
    use serde_json::json;

    #[test]
    fn facts_returns_literal_presence_and_default_source_from_supplied_generation() {
        let generation = Generation {
            number: 1,
            global: None,
            repo: Input {
                identity: "/project/.planning/config.v4.json".into(),
                bytes: None,
                stamp: None,
            },
            effective: Effective {
                raw_global: None,
                raw_repo: Some(json!({"roles":{"cad-executor":{"model":null}}})),
                global: json!({}),
                repo: json!({"roles":{"cad-executor":{"model":null}}}),
                values: json!({"roles":{"cad-executor":{"model":null,"effort":"high"}}}),
                sources: [("roles.cad-executor.model".into(), Layer::Repo)].into(),
                global_intent: false,
                diagnostics: Diagnostics::default(),
            },
        };
        let result = facts(&generation);
        let model = result
            .keys
            .iter()
            .find(|fact| fact.key == "roles.cad-executor.model")
            .unwrap();
        assert_eq!(
            (
                model.present_global,
                model.present_repo,
                model.stored_repo.clone(),
                model.effective.clone(),
                model.source.as_str()
            ),
            (false, true, Some(Value::Null), Value::Null, "repo")
        );
        let effort = result
            .keys
            .iter()
            .find(|fact| fact.key == "roles.cad-executor.effort")
            .unwrap();
        assert_eq!(
            (
                effort.present_global,
                effort.present_repo,
                effort.effective.clone(),
                effort.source.as_str()
            ),
            (false, false, json!("high"), "defaults")
        );
    }
}

#[cfg(test)]
mod routing_inputs_tests {
    use super::*;
    use crate::config::{Diagnostics, Effective, reload::Input};
    use serde_json::json;

    fn generation() -> Generation {
        Generation {
            number: 1,
            global: None,
            repo: Input {
                identity: "/project/.planning/config.v4.json".into(),
                bytes: None,
                stamp: None,
            },
            effective: Effective {
                raw_global: None,
                raw_repo: None,
                global: json!({}),
                repo: json!({}),
                values: json!({"model":{"escalate_on_failure":false},"roles":{"cad-executor":{"model":null,"effort":"high"}}}),
                sources: Default::default(),
                global_intent: false,
                diagnostics: Diagnostics::default(),
            },
        }
    }

    #[test]
    fn routing_inputs_captures_resolved_identities_and_exact_byte_digests() {
        let mut supplied = generation();
        supplied.repo.bytes = Some(b"{}".to_vec());
        supplied.repo.stamp = Some((11, 22, 33));
        supplied.global = Some(Input {
            identity: "/global/config.v4.json".into(),
            bytes: Some(Vec::new()),
            stamp: None,
        });
        supplied.effective.global_intent = true;
        assert_eq!(
            routing_inputs(&supplied),
            cadence::execution::model::ConfigInputs {
                repo: cadence::execution::model::ConfigInput {
                    identity: "/project/.planning/config.v4.json".into(),
                    content: Some(
                        "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a".into()
                    ),
                    stamp: Some((11, 22, 33)),
                },
                global: Some(cadence::execution::model::ConfigInput {
                    identity: "/global/config.v4.json".into(),
                    content: Some(
                        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into()
                    ),
                    stamp: None,
                }),
                global_alias: true,
            }
        );
    }

    #[test]
    fn role_input_reads_six_schema_defaults_without_treating_effective_values_as_stored() {
        for (role, effort) in [
            ("cad-planner", "high"),
            ("cad-assumptions-analyzer", "high"),
            ("cad-verifier", "high"),
            ("cad-reviewer", "medium"),
            ("cad-executor", "high"),
            ("cad-plan-checker", "low"),
        ] {
            let request = RouteRequest {
                role: role.into(),
                phase: None,
                plan: None,
                attempt: None,
            };
            let input = role_input(&generation(), &request).unwrap();
            assert_eq!(input.default_effort, effort);
            assert_eq!(input.role_effort, None);
            assert_eq!(input.role_model, None);
            assert_eq!(input.legacy_effort, None);
            assert_eq!(input.legacy_model, None);
            assert_eq!(input.attempt, 1);
            assert!(!input.escalate_on_failure);
        }
    }

    #[test]
    fn role_input_uses_projected_layer_and_source_for_each_winning_leaf() {
        let mut generation = generation();
        generation.effective.global =
            json!({"roles":{"cad-executor":{"model":null,"effort":null}}});
        generation.effective.repo =
            json!({"model":{"overrides":{"cad-executor":"opus"},"effort":{"cad-executor":"max"}}});
        generation.effective.sources = [
            ("roles.cad-executor.model".into(), Layer::Global),
            ("roles.cad-executor.effort".into(), Layer::Global),
            ("model.overrides.cad-executor".into(), Layer::Repo),
            ("model.effort.cad-executor".into(), Layer::Repo),
        ]
        .into();
        let request = RouteRequest {
            role: "cad-executor".into(),
            phase: None,
            plan: None,
            attempt: None,
        };
        let input = role_input(&generation, &request).unwrap();
        assert_eq!(
            input.role_model,
            Some(roles::Stored {
                key: "roles.cad-executor.model".into(),
                layer: "global".into(),
                value: Value::Null
            })
        );
        assert_eq!(
            input.role_effort,
            Some(roles::Stored {
                key: "roles.cad-executor.effort".into(),
                layer: "global".into(),
                value: Value::Null
            })
        );
        assert_eq!(
            input.legacy_model,
            Some(roles::Stored {
                key: "model.overrides.cad-executor".into(),
                layer: "repo".into(),
                value: json!("opus")
            })
        );
        assert_eq!(
            input.legacy_effort,
            Some(roles::Stored {
                key: "model.effort.cad-executor".into(),
                layer: "repo".into(),
                value: json!("max")
            })
        );
    }
}
