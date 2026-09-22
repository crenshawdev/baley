
use cadence::next_action::continuation::Decision;

#[test]
fn suite_repair_continuation_waits_on_plan_question() {
    use cadence::execution::history::{PlanProjection, SuiteRepairQuestion};
    let question = SuiteRepairQuestion { id: "suite-repair:run-1".into(), failed_run: "run-1".into(),
        failing_tests: vec!["repair::alpha".into()], proposed_paths: vec!["src/value.rs".into()] };
    let projection = PlanProjection { version: 3, worker_exits: vec![], round: None, completion: None, launches: vec!["run-1".into()], results: vec!["run-1".into()],
        relaunch: None, repair_question: Some(question.clone()), repair_answer: None, repair: None,
        outcome: "failed".into(), completed: false };
    assert_eq!(cadence::next_action::continuation::plan_repair_decision(&projection),
        Some(Decision::RepairSuite { question_id: question.id, approved: false }));
}
