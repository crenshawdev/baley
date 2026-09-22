use super::*;
use cadence::store::model::{ItemRecord, Origin, Snapshot, VERSION};

fn item(id: &str, text: &str) -> ItemRecord {
    ItemRecord {
        version: VERSION,
        id: id.into(),
        revision: 1,
        origin: Origin {
            source: "capture".into(),
            original: Evidence::Missing,
        },
        text: text.into(),
        kind: "note".into(),
        phase: None,
        disposition: Disposition::Captured,
        completed: false,
        filing_uncertain: false,
    }
}
fn view(items: Vec<ItemRecord>) -> View {
    View {
        items,
        decisions: vec![],
        snapshot: Snapshot::new(0, b"", b"", serde_json::Value::Null).unwrap(),
    }
}
fn prose(text: &str, line: usize) -> Candidate {
    Candidate {
        text: text.into(),
        item_id: None,
        provenance: Provenance::Document {
            path: "CONTEXT.md".into(),
            line,
            heading: "Durable decisions".into(),
            commit: None,
        },
    }
}

#[test]
fn frozen_token_relationships_and_raw_stopword_order() {
    for (a, b) in [
        ("seams", "seam"),
        ("closes", "close"),
        ("files", "file"),
        ("notes", "note"),
        ("types", "type"),
        ("changes", "change"),
        ("refused", "refuse"),
        ("removed", "remove"),
        ("running", "run"),
    ] {
        assert_eq!(rank::tokenize(a), rank::tokenize(b), "{a} / {b}");
    }
    assert_eq!(rank::tokenize("THE being its"), vec!["be", "it"]);
    assert_eq!(rank::tokenize("freed agreed"), vec!["freed", "agree"]);
    assert_ne!(rank::tokenize("verifies"), rank::tokenize("verify"));
    assert_ne!(rank::tokenize("indices"), rank::tokenize("index"));
}

#[test]
fn equal_scores_keep_candidate_order() {
    let candidates = (1..=7).map(|i| prose("quasar seam", i)).collect();
    let answer = Corpus::new(candidates, &BTreeSet::new()).query("quasar", None, "builtin").unwrap();
    for (i, hit) in answer.results.iter().enumerate() {
        assert_eq!(hit.provenance, prose("", i + 1).provenance);
    }
}

#[test]
fn a_repeated_query_term_does_not_change_the_answer() {
    let candidates = (1..=7).map(|i| prose("quasar seam", i)).collect();
    let corpus = Corpus::new(candidates, &BTreeSet::new());
    let answer = corpus.query("quasar", None, "builtin").unwrap();
    assert_eq!(
        answer,
        corpus.query("quasar quasar", None, "builtin").unwrap()
    );
    assert_eq!(answer, corpus.query("quasar", None, "builtin").unwrap());
}

#[test]
fn the_default_limit_is_5_while_total_counts_every_match() {
    let candidates = (1..=7).map(|i| prose("quasar seam", i)).collect();
    let answer = Corpus::new(candidates, &BTreeSet::new()).query("quasar", None, "builtin").unwrap();
    assert_eq!(answer.total, 7);
    assert_eq!(answer.results.len(), 5);
}

#[test]
fn an_explicit_limit_caps_the_results() {
    let candidates = (1..=7).map(|i| prose("quasar seam", i)).collect();
    assert_eq!(
        Corpus::new(candidates, &BTreeSet::new())
            .query("seams", Some(2), "builtin")
            .unwrap()
            .results
            .len(),
        2
    );
}

#[test]
fn a_non_positive_limit_is_refused() {
    let corpus = Corpus::new(vec![prose("quasar", 1)], &BTreeSet::new());
    for limit in [0, -1] {
        assert!(corpus.query("quasar", Some(limit), "builtin").is_err());
    }
}

#[test]
fn a_blank_query_is_refused() {
    let corpus = Corpus::new(vec![prose("quasar", 1)], &BTreeSet::new());
    assert!(corpus.query(" ", None, "builtin").is_err());
}

#[test]
fn an_unknown_backend_is_refused() {
    let corpus = Corpus::new(vec![prose("quasar", 1)], &BTreeSet::new());
    assert!(corpus.query("quasar", None, "other").is_err());
}

#[test]
fn backend_none_answers_empty_and_names_itself() {
    let corpus = Corpus::new(vec![prose("quasar", 1)], &BTreeSet::new());
    let disabled = corpus.query("quasar", None, "none").unwrap();
    assert_eq!(disabled.backend, "none");
    assert_eq!((disabled.total, disabled.results.len()), (0, 0));
}

#[test]
fn no_match_or_an_empty_corpus_answers_total_0() {
    let corpus = Corpus::new(vec![prose("quasar", 1)], &BTreeSet::new());
    assert_eq!(corpus.query("absent", None, "builtin").unwrap().total, 0);
    assert_eq!(
        Corpus::new(vec![], &BTreeSet::new())
            .query("quasar", None, "builtin")
            .unwrap()
            .total,
        0
    );
}

#[test]
fn declined_current_and_historical_candidates_never_affect_ranking_or_totals() {
    let old = item("declined", "quasar quasar");
    let historical = records(view(vec![old.clone()]).recall_items());
    let mut dead = old.clone();
    dead.revision = 2;
    dead.disposition = Disposition::Declined {
        reason: "quasar secret reason".into(),
    };
    let live = view(vec![old, dead, item("kept", "quasar")]);
    let mut candidates = current(&live);
    candidates.extend(historical);
    let actual = Corpus::new(candidates, &declined(&live))
        .query("quasar", None, "builtin")
        .unwrap();
    let expected = Corpus::new(
        records(view(vec![item("kept", "quasar")]).recall_items()),
        &BTreeSet::new(),
    )
    .query("quasar", None, "builtin")
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.total, 1);
}

use std::{fs, path::Path};

fn put(root: &Path, path: &str, text: &str) {
    let dest = root.join(path);
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(dest, text).unwrap();
}

#[test]
fn authored_sources_archives_and_receipts_are_eligible() {
    for path in [
        "PROJECT.md",
        "ROADMAP.md",
        "phases/1/SUMMARY.md",
        "phases/1/UAT.md",
        "phases/1/CONTEXT.md",
        "tasks/receipt/RECORD.md",
        "_archive-v1/2/CONTEXT.md",
    ] {
        assert!(documents::eligible(path), "{path}");
    }
}

#[test]
fn a_phase_context_offers_its_other_sections_and_not_its_local_decisions() {
    let snippets = documents::snippets(
        "phases/1/CONTEXT.md",
        "## Decisions\n- D-02 localquasar\n### Nested\nlocalquasar\n## Other\notherquasar\n## Durable decisions\n",
        None,
    );
    let corpus = Corpus::new(snippets, &BTreeSet::new());
    assert_eq!(corpus.query("otherquasar", None, "builtin").unwrap().total, 1);
    assert_eq!(
        corpus.query("localquasar", None, "builtin").unwrap().total,
        0
    );
}

#[test]
fn a_legacy_context_offers_its_decisions() {
    let legacy = documents::snippets("CONTEXT.md", "## Decisions\n- D-01 legacyquasar\n", None);
    assert_eq!(
        Corpus::new(legacy, &BTreeSet::new())
            .query("legacyquasar", None, "builtin")
            .unwrap()
            .total,
        1
    );
}

#[test]
fn ledgers_config_evidence_reports_credentials_and_traces_are_not_eligible() {
    for path in [
        "DECLINED.md",
        "FILED.md",
        "config.json",
        "source-evidence/PROJECT.md",
        "phases/1/reports/SUMMARY.md",
        "credentials.md",
        "trace.jsonl",
    ] {
        assert!(!documents::eligible(path), "{path}");
    }
}

#[test]
fn a_linked_or_escaping_entry_is_refused() {
    use documents::{Seen, Step, step};
    let link_file = Seen { link: true, contained: false, dir: false, file: false };
    let link_dir = Seen { link: true, contained: false, dir: false, file: false };
    let escaping = Seen { link: false, contained: false, dir: false, file: true };
    assert_eq!(step("phases/1/CONTEXT.md", &link_file), Step::Refuse);
    assert_eq!(step("phases/2", &link_dir), Step::Refuse);
    assert_eq!(step("phases/1/CONTEXT.md", &escaping), Step::Refuse);
}

#[test]
fn a_contained_directory_is_descended_and_a_contained_eligible_file_read() {
    use documents::{Seen, Step, step};
    let dir = Seen { link: false, contained: true, dir: true, file: false };
    let file = Seen { link: false, contained: true, dir: false, file: true };
    assert_eq!(step("phases/2", &dir), Step::Descend);
    assert_eq!(step("phases/1/CONTEXT.md", &file), Step::Read);
    assert_eq!(step("phases/1/reports/SUMMARY.md", &file), Step::Skip);
}

#[test]
fn unreadable_permitted_source_states_incomplete_coverage() {
    struct Denied;
    impl documents::ReadDocuments for Denied {
        fn list(&mut self, path: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
            documents::ReadDocuments::list(&mut documents::Files, path)
        }
        fn text(&mut self, _: &Path) -> std::io::Result<String> {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "PROJECT.md", "unreadablequasar");
    let docs = documents::read(dir.path(), &mut Denied);
    assert!(docs.candidates.is_empty());
    assert!(docs.incomplete[0].contains("PROJECT.md: source unavailable"));
}

fn history_answer(
    root: &Path,
    view: &View,
    query: &str,
    process: &mut dyn cadence::process::Process,
) -> Answer {
    let docs = documents::read(root, &mut documents::Files);
    let mut candidates = current(view);
    candidates.extend(docs.candidates);
    let history = history::read(root, view, &candidates, process);
    candidates.extend(history.candidates);
    let mut result = Corpus::new(candidates, &declined(view))
        .query(query, None, "builtin")
        .unwrap();
    result.incomplete = docs
        .incomplete
        .into_iter()
        .chain(history.incomplete)
        .collect();
    result
}

#[test]
fn an_archive_residue_line_yields_its_label_origin_and_phase_and_no_commit() {
    let candidates = history::residue(
        "ARCHIVE.md",
        "# Archive\n## release/with/slashes\n- `phases/1.10/SUMMARY.md`: residuefalcon is only a snippet\n",
        None,
    );
    assert_eq!(candidates.len(), 1);
    assert!(
        matches!(&candidates[0].provenance,Provenance::Residue {path,line:3,label,origin,phase,commit:None} if path == "ARCHIVE.md" && label == "release/with/slashes" && origin == "phases/1.10/SUMMARY.md" && phase == "1.10")
    );
}

#[test]
fn a_git_launch_failure_is_incomplete_coverage_while_archive_residue_still_answers() {
    struct Missing;
    impl cadence::process::Process for Missing {
        fn run(
            &mut self,
            _: &cadence::process::Launch,
        ) -> std::io::Result<cadence::process::Output> {
            Err(std::io::Error::other("git executable unavailable"))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "ARCHIVE.md",
        "# Archive\n## release/with/slashes\n- `phases/1.10/SUMMARY.md`: residuefalcon is only a snippet\n",
    );
    let history = history::read(dir.path(), &view(vec![]), &[], &mut Missing);
    assert!(
        history
            .incomplete
            .iter()
            .any(|r| r.contains("git executable unavailable"))
    );
    assert_eq!(
        Corpus::new(history.candidates, &BTreeSet::new())
            .query("residuefalcon", None, "builtin")
            .unwrap()
            .total,
        1
    );
}
