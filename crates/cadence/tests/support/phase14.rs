use crate::phase13::{Completed, tree};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

pub const OPEN: &str = "## Phases\n- [ ] **Phase 5: Legacy**\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";
pub const TICKED: &str = "## Phases\n- [x] **Phase 5: Legacy**\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";

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
