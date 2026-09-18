use crate::phase13::{Completed, apply, digest_of, git, query, tree, verify};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

pub const OPEN: &str = "## Phases\n- [ ] **Phase 5: Legacy**\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";
pub const TICKED: &str = "## Phases\n- [x] **Phase 5: Legacy**\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";

/// The adoption tree's roadmap: phases 1 to 3 ticked, 4 open.
pub const LEGACY: &str = "## Phases\n- [x] **Phase 1: First**\n- [x] **Phase 2: Second**\n- [x] **Phase 3: Third**\n- [ ] **Phase 4: Fourth**\n";
/// The same roadmap after the owner ticks phase 4 by hand.
pub const LEGACY_TICKED: &str = "## Phases\n- [x] **Phase 1: First**\n- [x] **Phase 2: Second**\n- [x] **Phase 3: Third**\n- [x] **Phase 4: Fourth**\n";

pub fn progress_fixture() -> Completed {
    let fixture = Completed::new();
    let root = fixture.project().join(".planning");
    fs::write(root.join("ROADMAP.md"), OPEN).unwrap();
    fs::create_dir_all(root.join("phases/5")).unwrap();
    fs::write(root.join("phases/5/PLAN-1.md"), "---\nphase: 5\nplan: 1\n---\n# Legacy plan\n\n## Tasks\n\n### Task 1: Deliver legacy work\n\n- **Files:** src/legacy.rs\n- **Action:** Deliver the legacy work.\n- **Verify:** cargo test\n").unwrap();
    fs::create_dir_all(root.join("deferred/5")).unwrap();
    fs::write(root.join("deferred/5/DEFERRED-diff-1.json"),
        r#"{"phase":"5","trigger":"diff","discriminator":"1","round":1,"findings":[{"description":"Review the legacy work"}]}"#).unwrap();
    fixture
}

/// A legacy tree no binary has touched: phase 1 derives complete (SUMMARY and
/// a passing UAT), phase 2 is ticked with one failing UAT item, phase 3 is
/// ticked with a plan only, phase 4 is open with a plan. One baseline commit.
pub fn legacy_fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(".planning");
    for phase in 1..=4 {
        fs::create_dir_all(root.join(format!("phases/{phase}"))).unwrap();
        fs::write(root.join(format!("phases/{phase}/PLAN-1.md")),
            format!("---\nphase: {phase}\nplan: 1\n---\n# Phase {phase} plan\n\n## Tasks\n\n### Task 1: Deliver phase {phase}\n\n- **Files:** src/phase{phase}.rs\n- **Action:** Deliver the phase.\n- **Verify:** cargo test\n")).unwrap();
    }
    fs::write(root.join("ROADMAP.md"), LEGACY).unwrap();
    fs::write(root.join("config.json"), "{}\n").unwrap();
    fs::write(root.join("phases/1/SUMMARY.md"), "# Phase 1 summary\n").unwrap();
    fs::write(root.join("phases/1/UAT.md"), "## Items\n\n### 1. Done\nstatus: pass\n").unwrap();
    fs::write(root.join("phases/2/SUMMARY.md"), "# Phase 2 summary\n").unwrap();
    fs::write(root.join("phases/2/UAT.md"), "## Items\n\n### 1. A\nstatus: pass\n\n### 2. B\nstatus: pass\n\n### 3. C\nstatus: fail\n").unwrap();
    git(temp.path(), &["init", "--initial-branch=fixture/adoption"]);
    fs::write(temp.path().join(".gitignore"), ".planning/\n").unwrap();
    git(temp.path(), &["add", ".gitignore"]);
    git(temp.path(), &["commit", "-m", "Adoption fixture baseline"]);
    temp
}

/// A second project verified and natively completed the way
/// phase13_verification completes one; the completion record's id comes back
/// with it.
pub fn natively_completed() -> (Completed, String) {
    let fixture = Completed::new();
    let project = fixture.project();
    let (accepted, _) = verify(project, "accepted", &[]);
    let basis = query(project, json!({"operation":"verification-read","phase":13}))["current"]["observed"].clone();
    assert!(basis.is_object(), "{basis}");
    let root = project.join(".planning");
    let requirements = root.join("REQUIREMENTS.md");
    let done = apply(project, json!({"operation":"verification-complete","request_id":"complete-13","attempt":accepted["id"],"basis":basis,
        "projections":{"roadmap":digest_of(&root.join("ROADMAP.md")),"requirements":requirements.exists().then(|| digest_of(&requirements))}}));
    assert_eq!(done["status"], "ok", "{done}");
    let id = done["receipt"]["record"]["id"].as_str().unwrap().to_owned();
    (fixture, id)
}

pub fn documents(project: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    tree(project).into_iter().filter_map(|(path, bytes)| {
        // Store journals and snapshots may change; every authored document and
        // the deferred input must remain byte-exact across the progress reads.
        let document = path.extension().is_some_and(|ext| ext == "md")
            || path.starts_with(".planning/deferred")
            || path == Path::new(".planning/config.json");
        document.then_some(bytes).flatten().map(|bytes| (path, bytes))
    }).collect()
}
