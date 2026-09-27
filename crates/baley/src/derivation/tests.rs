use super::*;

fn captured(text: &str) -> CapturedInputs {
    let parsed = parse_roadmap(text).unwrap();
    let mut phases: Vec<PhaseObservation> = Vec::new();
    for phase in &parsed.phases {
        if !phases
            .iter()
            .any(|p| p.relative_path == phase.relative_path)
        {
            phases.push(PhaseObservation {
                relative_path: phase.relative_path.clone(),
                plans: Observation::Absent,
            });
        }
    }
    CapturedInputs {
        root: "/planning".into(),
        root_probe: Observation::Present(()),
        roadmap: Observation::Present(text.as_bytes().into()),
        declarations: Some(Ok(parsed)),
        phases,
    }
}

#[test]
fn without_native_authority_plan_files_alone_decide_planned() {
    let mut capture = captured("## Phases\n- [ ] **Phase 1: One**");
    let phase = &derive(&capture).unwrap().phases[0];
    assert_eq!((phase.status, &phase.uat), (LifecycleStatus::Unplanned, &None));
    capture.phases[0].plans = Observation::Present(vec!["PLAN-1.md".into()]);
    let phase = &derive(&capture).unwrap().phases[0];
    assert_eq!((phase.status, &phase.uat), (LifecycleStatus::Planned, &None));
}

#[test]
fn ac1_failures_cannot_derive_success() {
    let mut capture = captured("## Phases\n- [ ] **Phase 1: One**");
    capture.root_probe = Observation::Absent;
    assert_eq!(
        derive(&capture).unwrap_err().code(),
        "missing-planning-root"
    );
    capture.root_probe = Observation::Present(());
    capture.roadmap = Observation::Absent;
    assert_eq!(derive(&capture).unwrap_err().code(), "missing-roadmap");
    capture.roadmap = Observation::Present(vec![]);
    capture.declarations = Some(parse_roadmap(""));
    assert_eq!(derive(&capture).unwrap_err().code(), "invalid-roadmap");
    let error = InputFailure {
        path: "/planning/phases/1".into(),
        category: InputFailureCategory::PermissionDenied,
        diagnostic: None,
    };
    capture.phases[0].plans = Observation::Failed(error.clone());
    assert_eq!(
        derive(&capture).unwrap_err(),
        DerivationError::InputFailure(error)
    );
}

/// An overlay in which the phase at `address` holds an applicable native
/// completion with one met truth.
fn complete(address: &str) -> AcceptanceOverlay {
    let mut overlay = AcceptanceOverlay::default();
    overlay.phases.insert(address.into(), AcceptancePhase { published: true, executed: true,
        completion: Some("c".into()), label: Some("complete".into()), met: 1, waived: 0, disagreement: None });
    overlay
}

#[test]
fn ac2_current_follows_list_order_not_number() {
    for order in [
        [8, 2, 5],
        [8, 5, 2],
        [2, 5, 8],
        [2, 8, 5],
        [5, 2, 8],
        [5, 8, 2],
    ] {
        let entries = order
            .map(|n| format!("- [ ] **Phase {n}: P{n}**"))
            .join("\n");
        let capture = captured(&format!("## Phases\n{entries}"));
        let answer = derive_with(&capture, &complete(&order[0].to_string())).unwrap();
        assert_eq!(answer.current, Some(PhaseId(f64::from(order[1]))));
        assert_eq!(answer.total, 3);
        assert_eq!(
            answer
                .phases
                .iter()
                .map(|p| p.id.number())
                .collect::<Vec<_>>(),
            order.map(f64::from)
        );
        assert_eq!(derive(&capture).unwrap().current, Some(PhaseId(f64::from(order[0]))));
    }
}

#[test]
fn ac2_numeric_ties_share_evidence_and_preserve_names_and_order() {
    let mut capture = captured(
        "## Phases\n- [ ] **Phase 2: Two**\n- [ ] **Phase 1.10: First tie**\n- [ ] **Phase 01: One**\n- [ ] **Phase 1.1: Second tie**",
    );
    capture.phases[1].plans = Observation::Present(vec!["PLAN-2.md".into()]);
    let answer = derive_with(&capture, &complete("2")).unwrap();
    assert_eq!(capture.phases.len(), 3);
    assert_eq!(answer.total, 4);
    assert_eq!(answer.current, Some(PhaseId(1.1)));
    assert_eq!(
        answer
            .phases
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["Two", "First tie", "One", "Second tie"]
    );
    assert_eq!(answer.phases[1].plans, answer.phases[3].plans);
    assert_eq!(answer.phases[1].status, LifecycleStatus::Planned);
    assert_eq!(answer.phases[3].status, LifecycleStatus::Planned);
}

#[test]
fn ac2_closed_null_zero_differs_from_live_null_all_complete() {
    let closed = derive(&captured("## Phases\nDone")).unwrap();
    assert_eq!(
        (closed.cycle, closed.current, closed.total),
        (Cycle::Closed, None, 0)
    );
    let capture = captured("## Phases\n- [ ] **Phase 1: One**");
    let live = derive_with(&capture, &complete("1")).unwrap();
    assert_eq!(
        (live.cycle, live.current, live.total),
        (Cycle::Live, None, 1)
    );
}

#[test]
fn a_ticked_checkbox_never_changes_the_derived_lifecycle() {
    let unchecked = captured("## Phases\n- [ ] **Phase 1: One**");
    let checked = captured("## Phases\n- [x] **Phase 1: One**");
    assert_eq!(derive(&unchecked).unwrap(), derive(&checked).unwrap());
    assert_eq!(
        derive(&checked).unwrap().phases[0].status,
        LifecycleStatus::Unplanned
    );
}

#[test]
fn a_roadmap_with_no_phases_section_or_only_malformed_entries_is_invalid() {
    let unchecked = captured("## Phases\n- [ ] **Phase 1: One**");
    for text in ["# No section", "## Phases\n- Phase 1: Bad"] {
        let mut capture = unchecked.clone();
        capture.roadmap = Observation::Present(text.as_bytes().into());
        capture.declarations = Some(parse_roadmap(text));
        assert_eq!(derive(&capture).unwrap_err().code(), "invalid-roadmap");
    }
}

#[test]
fn a_malformed_entry_beside_a_valid_one_is_skipped() {
    assert_eq!(
        derive(&captured(
            "## Phases\n- Phase 8: Bad\n- [ ] **Phase 1: Good**"
        ))
        .unwrap()
        .total,
        1
    );
}

#[test]
fn a_bom_and_crlf_line_endings_parse_without_shifting_source_lines() {
    for text in [
        "## Phases\n- [ ] **Phase 1: One**",
        "\u{feff}## Phases\r\n- [ ] **Phase 1: One**\r\n",
    ] {
        let parsed = parse_roadmap(text).unwrap();
        assert_eq!(parsed.phases.len(), 1);
        assert_eq!(parsed.phases[0].source_line, 2);
    }
}

#[test]
fn malformed_or_misplaced_phases_sections_are_refused() {
    for text in [
        "## Phases\r- [ ] **Phase 1: One**",
        "# Roadmap",
        "```\n## Phases\n```",
        "## Phases\n- [X] **Phase 1: One**",
        "## Phases\n## Details\n### Phase 1: One",
        "## Phases\n## Later\n- [ ] **Phase 1: One**",
    ] {
        assert!(parse_roadmap(text).is_err(), "{text:?}");
    }
}

#[test]
fn a_phases_section_with_no_entry_lines_is_a_closed_cycle() {
    for text in [
        "## Phases",
        "  ## Phases  \nAll done\n## Details\nphase 1 is prose",
        "## Phases\n```\nPhase 1\n```",
        "## Phases\nNotPhase 1\nPhase 1abc",
    ] {
        assert_eq!(
            parse_roadmap(text).unwrap().cycle,
            Cycle::Closed,
            "{text:?}"
        );
    }
}

#[test]
fn only_entries_inside_the_phases_section_count_and_a_trailing_text_is_the_description() {
    let parsed = parse_roadmap("## Phases\n- [X] **Phase 8: Bad**\n- [ ] **Phase 2: Good** - description\n## Details\n- [ ] **Phase 3: Outside**").unwrap();
    assert_eq!(parsed.phases.len(), 1);
    assert_eq!(parsed.phases[0].description, "description");
}

#[test]
fn parsers_roadmap_fences_match_character_length_and_empty_info() {
    let text = "~~~example\n## Phases\n- [ ] **Phase 99: Fake**\n~~~\n   ## Phases\n````rust\n- [ ] **Phase 90: Fake**\n```\n## Fake boundary\n```` not a closer\n- [ ] **Phase 91: Fake**\n~~~~\n`````   \n- [x] **Phase 2: Real**\n   ~~~\n- [ ] **Phase 92: Fake**\n   ~~~\n- [ ] **Phase 1: First**";
    let parsed = parse_roadmap(text).unwrap();
    assert_eq!(
        parsed
            .phases
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["Real", "First"]
    );
    assert!(parsed.phases[0].checked);
    // Four spaces do not open a fence in the frozen scanner.
    assert_eq!(
        parse_roadmap("## Phases\n    ```\n- [ ] **Phase 1: Real**")
            .unwrap()
            .phases
            .len(),
        1
    );
}

#[test]
fn entries_keep_their_textual_order_as_ordinals() {
    let parsed = parse_roadmap("## Phases\n- [ ] **Phase 2: Two**\n- [ ] **Phase 1.10: Decimal A**\n- [ ] **Phase 01: Alias A**\n- [ ] **Phase 1.1: Decimal B**\n- [ ] **Phase 1.0: Alias B**\n- [ ] **Phase 1: Alias C**").unwrap();
    assert_eq!(
        parsed.phases.iter().map(|p| p.ordinal).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5]
    );
}

#[test]
fn an_address_is_the_number_as_javascript_formats_it_and_names_its_directory() {
    let parsed = parse_roadmap("## Phases\n- [ ] **Phase 2: Two**\n- [ ] **Phase 1.10: Decimal A**\n- [ ] **Phase 01: Alias A**\n- [ ] **Phase 1.1: Decimal B**\n- [ ] **Phase 1.0: Alias B**\n- [ ] **Phase 1: Alias C**").unwrap();
    assert_eq!(
        parsed
            .phases
            .iter()
            .map(|p| p.id.address())
            .collect::<Vec<_>>(),
        ["2", "1.1", "1", "1.1", "1", "1"]
    );
    for p in parsed.phases {
        assert_eq!(
            p.relative_path,
            std::path::PathBuf::from(format!("phases/{}", p.id.address()))
        );
    }
    for (number, address) in [
        ("0.000001", "0.000001"),
        ("0.0000001", "1e-7"),
        ("1000000000000000000000", "1e+21"),
        ("9007199254740993", "9007199254740992"),
    ] {
        let parsed = parse_roadmap(&format!("## Phases\n- [ ] **Phase {number}: N**")).unwrap();
        assert_eq!(parsed.phases[0].id.address(), address);
    }
}

fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = serde_json::to_vec(value).unwrap();
    assert_eq!(&serde_json::from_slice::<T>(&bytes).unwrap(), value);
}

#[test]
fn contract_exactly_four_native_status_strings() {
    for (status, word) in [
        (LifecycleStatus::Unplanned, "unplanned"),
        (LifecycleStatus::Planned, "planned"),
        (LifecycleStatus::Executed, "executed"),
        (LifecycleStatus::Complete, "complete"),
    ] {
        assert_eq!(serde_json::to_value(status).unwrap(), word);
        round_trip(&status);
    }
    for word in [
        "paused",
        "Complete",
        "ready to plan",
        "context gathered",
        "phase complete",
    ] {
        assert!(serde_json::from_value::<LifecycleStatus>(word.into()).is_err());
    }
}

#[test]
fn contract_round_trips_order_plans_counters_and_null_cycles() {
    let closed = Lifecycle {
        cycle: Cycle::Closed,
        current: None,
        total: 0,
        phases: vec![],
    };
    let mut live = Lifecycle {
        cycle: Cycle::Live,
        ..closed.clone()
    };
    for (i, plans) in [vec![], vec!["PLAN-1.md"], vec!["PLAN-10.md", "PLAN-2.md"]]
        .into_iter()
        .enumerate()
    {
        live.phases.push(PhaseRecord {
            id: PhaseId((3 - i) as f64),
            name: format!("entry {i}"),
            plans: plans.into_iter().map(str::to_owned).collect(),
            status: LifecycleStatus::Complete,
            uat: (i != 0).then_some(UatCounts {
                pass: 1,
                fail: 2,
                pending: 3,
                skipped: 4,
                blocked: 5,
            }),
        });
    }
    live.total = live.phases.len();
    assert_ne!(closed, live);
    round_trip(&closed);
    round_trip(&live);
    assert_eq!(
        serde_json::to_value(&closed).unwrap()["current"],
        serde_json::Value::Null
    );
    assert_eq!(
        serde_json::to_value(&live).unwrap()["current"],
        serde_json::Value::Null
    );
}

fn conflict_only<T>(
    result: &Result<T, DerivationError>,
    source: &str,
    field: &str,
    declared: &str,
    derived: &str,
) -> bool {
    matches!(result, Err(DerivationError::StateConflict { source: s, field: f, declared: a, derived: b, .. })
        if s == source && f == field && a == declared && b == derived && a != b)
}

#[test]
fn ac6_conflicts_both_checkbox_directions() {
    for checked in [true, false] {
        let capture = captured(&format!(
            "## Phases\n- [{}] **Phase 3: Three**",
            if checked { "x" } else { " " }
        ));
        let overlay = if checked { AcceptanceOverlay::default() } else { complete("3") };
        let answer = derive_with(&capture, &overlay).unwrap();
        let result = check_consistency(validate_inputs(&capture).unwrap(), &answer);
        let declared = checked.to_string();
        let derived = (!checked).to_string();
        assert!(conflict_only(
            &result,
            "ROADMAP.md:2 entry 0",
            "complete",
            &declared,
            &derived
        ));
        let error = result.unwrap_err().to_string();
        for expected in [
            "state-conflict",
            "ROADMAP.md:2 entry 0",
            &declared,
            &derived,
        ] {
            assert!(error.contains(expected));
        }
    }
}

/// A checked answer over one unticked phase 3, and the memo it would publish.
fn checked_with_memo() -> (RecheckedLifecycle, LifecycleMemo) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ROADMAP.md"), "## Phases\n- [ ] **Phase 3: Three**").unwrap();
    let checked = query(temp.path(), &mut ArtifactFiles).unwrap();
    let key = input_key_with(checked.capture(), checked.overlay()).unwrap();
    let memo = LifecycleMemo::fresh(key, checked.answer().clone());
    (checked, memo)
}

#[test]
fn installing_the_memo_keeps_unrelated_data_and_the_namespace() {
    let (checked, memo) = checked_with_memo();
    let original = serde_json::json!({"unrelated":[1,null], "derivation":{"extension":true}});
    let installed = checked.with_memo(&original, &memo).unwrap();
    assert_eq!(installed["unrelated"], original["unrelated"]);
    assert_eq!(installed["derivation"]["extension"], true);
    assert_eq!(installed["derivation"]["memo"], serde_json::to_value(&memo).unwrap());
}

#[test]
fn installing_the_memo_into_empty_data_creates_only_the_derivation_namespace() {
    let (checked, memo) = checked_with_memo();
    let installed = checked.with_memo(&serde_json::Value::Null, &memo).unwrap();
    assert_eq!(installed.as_object().unwrap().keys().collect::<Vec<_>>(), ["derivation"]);
    assert_eq!(installed["derivation"].as_object().unwrap().keys().collect::<Vec<_>>(), ["memo"]);
}

#[test]
fn a_malformed_derivation_namespace_is_refused_rather_than_overwritten() {
    let (checked, memo) = checked_with_memo();
    for data in [
        serde_json::json!(42),
        serde_json::json!([]),
        serde_json::json!({"derivation":null}),
        serde_json::json!({"derivation":[]}),
    ] {
        assert_eq!(checked.with_memo(&data, &memo).unwrap_err().code(), "derivation-conflict", "{data}");
    }
}

fn key_fixture() -> CapturedInputs {
    let mut c = captured("## Phases\n- [ ] **Phase 1: One**\n- [ ] **Phase 2: Two**\n");
    c.phases[0].plans = Observation::Present(vec!["PLAN-1.md".into(), "PLAN-2.md".into()]);
    c
}

fn key_rows() -> Vec<(&'static str, CapturedInputs, CapturedInputs)> {
    let base = key_fixture();
    let mut rows = Vec::new();
    macro_rules! row {
        ($name:literal, $c:ident, $change:expr) => {{
            let mut $c = base.clone();
            $change;
            rows.push(($name, base.clone(), $c));
        }};
    }
    row!("root address", c, c.root = "/other".into());
    row!("root probe", c, c.root_probe = Observation::Absent);
    row!("roadmap outcome", c, c.roadmap = Observation::Absent);
    row!(
        "roadmap bytes",
        c,
        c.roadmap = Observation::Present(b"other".to_vec())
    );
    row!("phase count", c, {
        c.declarations
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .phases
            .pop();
    });
    row!(
        "phase order",
        c,
        c.declarations
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .phases
            .reverse()
    );
    row!(
        "phase id",
        c,
        c.declarations.as_mut().unwrap().as_mut().unwrap().phases[0].id = PhaseId(3.0)
    );
    row!("phase path", c, {
        c.declarations.as_mut().unwrap().as_mut().unwrap().phases[0].relative_path =
            "phases/other".into();
        c.phases[0].relative_path = "phases/other".into();
    });
    row!(
        "listing outcome",
        c,
        c.phases[0].plans = Observation::Absent
    );
    row!(
        "plan count",
        c,
        c.phases[0].plans = Observation::Present(vec!["PLAN-1.md".into()])
    );
    row!(
        "plan name",
        c,
        c.phases[0].plans = Observation::Present(vec!["PLAN-1.md".into(), "PLAN-3.md".into()])
    );
    for target in ["root", "roadmap", "listing"] {
        let variants = (0..7)
            .map(|n| {
                let mut c = base.clone();
                fn variant<T: Default>(n: usize) -> Observation<T> {
                    match n {
                        0 => Observation::Absent,
                        1 => Observation::Present(T::default()),
                        _ => Observation::Failed(InputFailure {
                            path: "/ignored".into(),
                            diagnostic: Some("ignored".into()),
                            category: [
                                InputFailureCategory::PermissionDenied,
                                InputFailureCategory::NotDirectory,
                                InputFailureCategory::InvalidPath,
                                InputFailureCategory::SymlinkLoop,
                                InputFailureCategory::OtherIo,
                            ][n - 2],
                        }),
                    }
                }
                match target {
                    "root" => c.root_probe = variant(n),
                    "roadmap" => c.roadmap = variant(n),
                    _ => c.phases[0].plans = variant(n),
                }
                c
            })
            .collect::<Vec<_>>();
        for a in 0..variants.len() {
            for b in a + 1..variants.len() {
                rows.push((target, variants[a].clone(), variants[b].clone()));
            }
        }
    }
    rows
}

fn key_table_failures(encoder: impl Fn(&CapturedInputs) -> String) -> Vec<&'static str> {
    key_rows()
        .iter()
        .filter_map(|(name, a, b)| (encoder(a) == encoder(b)).then_some(*name))
        .collect()
}

#[test]
fn every_captured_field_and_outcome_category_changes_the_key() {
    assert!(key_table_failures(|c| input_key(c).unwrap()).is_empty());
}

#[test]
fn a_domain_encoding_or_semantics_version_change_changes_the_encoding() {
    let c = key_fixture();
    let original = encode_inputs(&c).unwrap();
    for (domain, encoding, semantics) in [
        ("other", ENCODING_VERSION, SEMANTICS_VERSION),
        (DOMAIN, ENCODING_VERSION + 1, SEMANTICS_VERSION),
        (DOMAIN, ENCODING_VERSION, SEMANTICS_VERSION + 1),
    ] {
        assert_ne!(
            crate::store::model::digest(&original),
            crate::store::model::digest(
                &memo::encode_versioned(&c, domain, encoding, semantics).unwrap()
            )
        );
    }
}

#[test]
fn plan_listing_order_does_not_change_the_key() {
    let c = key_fixture();
    let mut reversed = c.clone();
    if let Observation::Present(names) = &mut reversed.phases[0].plans {
        names.reverse();
    }
    assert_eq!(input_key(&c), input_key(&reversed));
}

#[test]
fn a_failed_root_probe_refuses_derivation() {
    for (_, a, _) in key_rows() {
        if let Observation::Failed(_) = a.root_probe {
            assert!(derive(&a).is_err());
        }
    }
}

#[test]
fn the_v2_encoding_of_a_minimal_capture_is_the_fixed_bytes_and_key() {
    let mut c = captured("## Phases\n");
    c.root = "/p".into();
    let expected = "0000000000000011636164656e63652e6c6966656379636c650000000000000002000000000000000300000000000000022f700101000000000000000a2323205068617365730a0000000000000000";
    let bytes = encode_inputs(&c).unwrap();
    assert_eq!(
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        expected
    );
    assert_eq!(
        input_key(&c).unwrap(),
        "4e3217beae0d07c20320d71147cb29fa341a2079830bb58c94eee8652bd340b5"
    );
}

#[test]
fn length_prefixed_fields_keep_ab_c_distinct_from_a_bc() {
    let mut a = key_fixture();
    let mut b = a.clone();
    a.phases[0].plans = Observation::Present(vec!["ab".into(), "c".into()]);
    b.phases[0].plans = Observation::Present(vec!["a".into(), "bc".into()]);
    assert_ne!(input_key(&a), input_key(&b));
}

#[test]
fn declaration_order_changes_the_key() {
    let a = key_fixture();
    let mut b = a.clone();
    b.declarations
        .as_mut()
        .unwrap()
        .as_mut()
        .unwrap()
        .phases
        .reverse();
    assert_ne!(input_key(&a), input_key(&b));
}

/// The memo of the key fixture with phase 1 natively complete, so the first
/// row carries counts and the second none.
fn memo_fixture() -> (String, Lifecycle, serde_json::Value) {
    let c = key_fixture();
    let mut overlay = complete("1");
    overlay.phases.get_mut("1").unwrap().waived = 1;
    let answer = derive_with(&c, &overlay).unwrap();
    let key = input_key_with(&c, &overlay).unwrap();
    let raw = serde_json::to_value(LifecycleMemo::fresh(key.clone(), answer.clone())).unwrap();
    (key, answer, raw)
}
fn memo_corruptions() -> Vec<(String, serde_json::Value)> {
    use serde_json::json;
    let (_, _, raw) = memo_fixture();
    let mut rows = Vec::new();
    for (pointer, field, value) in [
        ("/cycle", "cycle", json!("closed")),
        ("/current", "current", json!(null)),
        ("/total", "total", json!(3)),
        ("/phases/1/id", "phases[1].id", json!(3.0)),
        ("/phases/0/name", "phases[0].name", json!("Changed")),
        (
            "/phases/0/plans",
            "phases[0].plans",
            json!(["PLAN-2.md", "PLAN-1.md"]),
        ),
        ("/phases/0/status", "phases[0].status", json!("executed")),
        ("/phases/0/uat", "phases[0].uat", json!(null)),
        (
            "/phases/1/uat",
            "phases[1].uat",
            json!({"pass":0,"fail":0,"pending":0,"skipped":0,"blocked":0}),
        ),
    ] {
        let mut changed = raw.clone();
        *changed["answer"].pointer_mut(pointer).unwrap() = value;
        rows.push((field.into(), changed));
    }
    for counter in ["pass", "fail", "pending", "skipped", "blocked"] {
        let mut changed = raw.clone();
        changed["answer"]["phases"][0]["uat"][counter] = json!(9);
        rows.push((format!("phases[0].uat.{counter}"), changed));
    }
    let mut changed = raw;
    changed["answer"]["phases"]
        .as_array_mut()
        .unwrap()
        .reverse();
    rows.push(("phases[0].id".into(), changed));
    rows
}

#[test]
fn an_absent_memo_misses_and_an_identical_one_hits() {
    let (key, fresh, raw) = memo_fixture();
    assert_eq!(check_memo(None, &key, &fresh), Ok(MemoDisposition::Miss));
    assert_eq!(
        check_memo(Some(&raw), &key, &fresh),
        Ok(MemoDisposition::Hit)
    );
}

#[test]
fn a_differing_answer_field_conflicts_naming_the_field_and_both_hashes() {
    let (key, fresh, _) = memo_fixture();
    for (field, raw) in memo_corruptions() {
        assert!(
            matches!(check_memo(Some(&raw), &key, &fresh), Err(DerivationError::DerivationConflict { requested_hash, stored_hash, fields }) if requested_hash == key && stored_hash.as_deref() == Some(&key) && fields.contains(&field)),
            "{field}"
        );
    }
}

#[test]
fn a_changed_version_or_input_hash_misses() {
    let (key, fresh, raw) = memo_fixture();
    for name in ["encoding_version", "semantics_version", "input_hash"] {
        let mut changed = raw.clone();
        changed[name] = match name {
            "input_hash" => serde_json::json!("a".repeat(64)),
            "encoding_version" => serde_json::json!(ENCODING_VERSION + 1),
            _ => serde_json::json!(SEMANTICS_VERSION + 1),
        };
        assert_eq!(
            check_memo(Some(&changed), &key, &fresh),
            Ok(MemoDisposition::Miss)
        );
    }
}

#[test]
fn a_malformed_or_missing_envelope_field_conflicts_naming_that_field() {
    use serde_json::json;
    let (key, fresh, raw) = memo_fixture();
    let mut rows = vec![
        ("encoding_version", serde_json::Value::Null),
        ("encoding_version", json!({})),
    ];
    for name in [
        "encoding_version",
        "semantics_version",
        "input_hash",
        "answer",
    ] {
        let mut changed = raw.clone();
        changed.as_object_mut().unwrap().remove(name);
        rows.push((name, changed));
        let mut changed = raw.clone();
        changed[name] = json!(false);
        rows.push((name, changed));
    }
    for (field, raw) in rows {
        assert!(
            matches!(check_memo(Some(&raw), &key, &fresh), Err(DerivationError::DerivationConflict { fields, .. }) if fields == [field])
        );
    }
}

#[test]
fn a_malformed_answer_field_conflicts_naming_its_path() {
    use serde_json::json;
    let (_, fresh, raw) = memo_fixture();
    for (field, value) in [
        ("status", json!("paused")),
        ("id", json!(null)),
        ("uat", json!({})),
        ("plans", json!(false)),
    ] {
        let mut changed = raw.clone();
        changed["answer"]["phases"][0][field] = value;
        let error = check_memo(Some(&changed), &"b".repeat(64), &fresh).unwrap_err();
        assert_eq!(error.code(), "derivation-conflict");
        assert!(error.to_string().contains(&format!("phases[0].{field}")));
    }
}

#[test]
fn an_older_semantics_version_misses_unread_while_a_malformed_hash_still_refuses() {
    use serde_json::json;
    let (key, fresh, raw) = memo_fixture();
    let mut old = raw.clone();
    old["semantics_version"] = json!(SEMANTICS_VERSION + 1);
    old["answer"] = json!({"opaque":"older schema"});
    assert_eq!(
        check_memo(Some(&old), &key, &fresh),
        Ok(MemoDisposition::Miss)
    );
    old["input_hash"] = json!("broken");
    assert!(check_memo(Some(&old), &key, &fresh).is_err());
}

#[test]
fn a_malformed_derivation_namespace_refuses_and_an_absent_one_reads_as_none() {
    use serde_json::json;
    let (key, _, _) = memo_fixture();
    for data in [
        json!({"derivation":null}),
        json!({"derivation":[]}),
        json!([]),
    ] {
        assert_eq!(
            memo_from_data(&data, &key).unwrap_err().code(),
            "derivation-conflict"
        );
    }
    assert!(memo_from_data(&json!({}), &key).unwrap().is_none());
}

#[test]
fn a_phase_number_beyond_f64_range_survives_json_and_its_memo_hits() {
    // Overflow is still a numeric identity in the domain and must survive JSON.
    let c = captured(&format!(
        "## Phases\n- [ ] **Phase {}: Overflow**",
        "9".repeat(400)
    ));
    let fresh = derive(&c).unwrap();
    round_trip(&fresh);
    let key = input_key(&c).unwrap();
    let memo = serde_json::to_value(LifecycleMemo::fresh(key.clone(), fresh.clone())).unwrap();
    assert_eq!(
        check_memo(Some(&memo), &key, &fresh),
        Ok(MemoDisposition::Hit)
    );
}

#[test]
fn each_io_error_is_the_failure_category_it_names_and_keeps_its_path() {
    use std::io::{Error, ErrorKind};
    let path = std::path::Path::new("/planning/phases/1/UAT.md");
    for (error, category) in [
        (Error::from_raw_os_error(libc::ELOOP), InputFailureCategory::SymlinkLoop),
        (Error::from_raw_os_error(libc::ENAMETOOLONG), InputFailureCategory::InvalidPath),
        (Error::from(ErrorKind::InvalidInput), InputFailureCategory::InvalidPath),
        (Error::from(ErrorKind::PermissionDenied), InputFailureCategory::PermissionDenied),
        (Error::from(ErrorKind::NotADirectory), InputFailureCategory::NotDirectory),
        (Error::other("artifact is not a regular readable file"), InputFailureCategory::OtherIo),
    ] {
        let failure = capture::failure(path, error);
        assert_eq!((failure.path.as_path(), failure.category), (path, category));
    }
}

#[test]
fn not_found_is_absent_and_no_other_error_is_ever_taken_for_absence() {
    use std::io::{Error, ErrorKind};
    let path = std::path::Path::new("/planning/phases/1/SUMMARY.md");
    assert_eq!(capture::observation(path, Ok(())), Observation::Present(()));
    assert_eq!(capture::observation::<()>(path, Err(Error::from(ErrorKind::NotFound))), Observation::Absent);
    assert!(matches!(
        capture::observation::<()>(path, Err(Error::from(ErrorKind::PermissionDenied))),
        Observation::Failed(failure) if failure.category == InputFailureCategory::PermissionDenied
    ));
}

#[test]
fn only_plan_dash_ascii_digits_md_are_plans() {
    for name in ["PLAN-1.md", "PLAN-02.md", "PLAN-123.md"] {
        assert!(capture::admitted(name), "{name}");
    }
    for name in ["PLAN.md", "plan.md", "PLAN-.md", "PLAN-1a.md", "PLAN-١.md", "PLAN-1.md.bak", "PLAN-1.MD", "PLAN-1", "SUMMARY.md"] {
        assert!(!capture::admitted(name), "{name}");
    }
}

#[test]
fn a_root_is_normalized_from_its_text_dropping_dot_and_popping_dot_dot() {
    use std::path::{Path, PathBuf};
    for (selected, normalized) in [
        ("/project/./.planning", "/project/.planning"),
        ("/project/src/../.planning", "/project/.planning"),
        ("/project/.planning/phases/..", "/project/.planning"),
        ("/..", "/"),
    ] {
        assert_eq!(capture::normalize(Path::new(selected)), PathBuf::from(normalized), "{selected}");
    }
}

#[test]
fn the_first_disagreeing_entry_is_refused_even_when_two_entries_share_an_address() {
    let capture = captured("## Phases\n- [ ] **Phase 3.0: First**\n- [x] **Phase 3: Second**");
    let answer = derive(&capture).unwrap();
    let result = check_consistency(validate_inputs(&capture).unwrap(), &answer);
    assert!(conflict_only(&result, "ROADMAP.md:3 entry 1", "complete", "true", "false"), "{result:?}");
}
