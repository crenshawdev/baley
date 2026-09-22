use std::path::Path;

#[test]
pub(crate) fn rendered_skill_files_are_protected() {
    let root = Path::new("/fixture");
    assert_eq!(cadence::execution::render::RENDERED_PROJECT_FILES.len(), 24);
    for rendered in cadence::execution::render::RENDERED_PROJECT_FILES {
        let relative = rendered.path;
        assert!(
            super::protected_target(&root.join(relative)).unwrap(),
            "{relative} must be protected as a binary-rendered project file"
        );
    }
    assert!(!super::protected_target(&root.join("skills/authored/SKILL.md")).unwrap());
}
