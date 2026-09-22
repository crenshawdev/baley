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
fn deterministic_ties_deduplicated_query_default_limit_and_total() {
    let candidates = (1..=7).map(|i| prose("quasar seam", i)).collect();
    let corpus = Corpus::new(candidates, &BTreeSet::new());
    let answer = corpus.query("quasar", None, "builtin").unwrap();
    assert_eq!(answer.total, 7);
    assert_eq!(answer.results.len(), 5);
    for (i, hit) in answer.results.iter().enumerate() {
        assert_eq!(hit.provenance, prose("", i + 1).provenance);
    }
    assert_eq!(
        answer,
        corpus.query("quasar quasar", None, "builtin").unwrap()
    );
    assert_eq!(answer, corpus.query("quasar", None, "builtin").unwrap());
    assert_eq!(
        corpus
            .query("seams", Some(2), "builtin")
            .unwrap()
            .results
            .len(),
        2
    );
}

#[test]
fn empty_zero_hits_invalid_limits_disabled_and_unknown() {
    let corpus = Corpus::new(vec![prose("quasar", 1)], &BTreeSet::new());
    for limit in [0, -1] {
        assert!(corpus.query("quasar", Some(limit), "builtin").is_err());
    }
    assert!(corpus.query(" ", None, "builtin").is_err());
    assert!(corpus.query("quasar", None, "other").is_err());
    let disabled = corpus.query("quasar", None, "none").unwrap();
    assert_eq!(disabled.backend, "none");
    assert_eq!((disabled.total, disabled.results.len()), (0, 0));
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

fn temp() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        !dir.path().starts_with(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
        )
    );
    dir
}
fn put(root: &Path, path: &str, text: &str) {
    let dest = root.join(path);
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(dest, text).unwrap();
}

#[test]
fn authored_sources_archives_receipts_and_context_selection_are_explicit() {
    let dir = temp();
    for (path, term) in [
        ("PROJECT.md", "projectquasar"),
        ("ROADMAP.md", "roadmapquasar"),
        ("phases/1/SUMMARY.md", "summaryquasar"),
        ("phases/1/UAT.md", "uatquasar"),
        ("tasks/receipt/RECORD.md", "recordquasar"),
        ("_archive-v1/2/CONTEXT.md", "archivequasar"),
    ] {
        put(dir.path(), path, &format!("# Evidence\n\n{term}\n"));
    }
    put(
        dir.path(),
        "phases/1/CONTEXT.md",
        "## Decisions\n- D-02 localquasar\n### Nested\nlocalquasar\n## Other\notherquasar\n## Durable decisions\n",
    );
    let docs = documents::read(dir.path(), &mut documents::Files);
    assert!(docs.incomplete.is_empty());
    let corpus = Corpus::new(docs.candidates, &BTreeSet::new());
    for term in [
        "projectquasar",
        "roadmapquasar",
        "summaryquasar",
        "uatquasar",
        "recordquasar",
        "archivequasar",
        "otherquasar",
    ] {
        assert_eq!(
            corpus.query(term, None, "builtin").unwrap().total,
            1,
            "{term}"
        );
    }
    assert_eq!(
        corpus.query("localquasar", None, "builtin").unwrap().total,
        0
    );
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
fn disallowed_files_and_escaping_links_cannot_supply_hits() {
    use std::os::unix::fs::symlink;
    let dir = temp();
    let outside = temp();
    put(outside.path(), "CONTEXT.md", "escapedquasar");
    for path in [
        "DECLINED.md",
        "FILED.md",
        "config.json",
        "source-evidence/PROJECT.md",
        "phases/1/reports/SUMMARY.md",
        "credentials.md",
        "trace.jsonl",
    ] {
        put(dir.path(), path, "forbiddenquasar");
    }
    fs::create_dir_all(dir.path().join("phases/1")).unwrap();
    symlink(
        outside.path().join("CONTEXT.md"),
        dir.path().join("phases/1/CONTEXT.md"),
    )
    .unwrap();
    symlink(outside.path(), dir.path().join("phases/2")).unwrap();
    let docs = documents::read(dir.path(), &mut documents::Files);
    assert_eq!(docs.incomplete.len(), 2);
    let corpus = Corpus::new(docs.candidates, &BTreeSet::new());
    assert_eq!(
        corpus
            .query("escapedquasar forbiddenquasar", None, "builtin")
            .unwrap()
            .total,
        0
    );
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
    let dir = temp();
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
fn residue_preserves_label_origin_and_absence_of_invented_history() {
    struct Missing;
    impl cadence::process::Process for Missing {
        fn run(
            &mut self,
            _: &cadence::process::Launch,
        ) -> std::io::Result<cadence::process::Output> {
            Err(std::io::Error::other("git executable unavailable"))
        }
    }
    let dir = temp();
    put(
        dir.path(),
        "ARCHIVE.md",
        "# Archive\n## release/with/slashes\n- `phases/1.10/SUMMARY.md`: residuefalcon is only a snippet\n",
    );
    let answer = history_answer(dir.path(), &view(vec![]), "residuefalcon", &mut Missing);
    assert_eq!(answer.total, 1);
    assert!(
        answer
            .incomplete
            .iter()
            .any(|r| r.contains("git executable unavailable"))
    );
    assert!(
        matches!(&answer.results[0].provenance,Provenance::Residue {path,line:3,label,origin,phase,commit:None} if path == "ARCHIVE.md" && label == "release/with/slashes" && origin == "phases/1.10/SUMMARY.md" && phase == "1.10")
    );
}
