//! The decisions here run over a table of what each lookup answers for one
//! exact path, and a path not in the table is missing. The table resolves
//! nothing: a link names the canonical path it leads to. Nothing here touches
//! a disk, reads the environment or starts a program.

use super::{
    Entry, Lookup, Part, ResolveFailure, contains, is_inside, resolve_existing_prefix,
    resolve_target,
};
use std::collections::BTreeMap;
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ask {
    Metadata,
    Present,
    Canonicalize,
}

struct Seen {
    canonical: PathBuf,
    entry: Entry,
}

struct Tree {
    paths: BTreeMap<PathBuf, Seen>,
    /// One lookup of one path that fails with something other than missing.
    failure: Option<(Ask, PathBuf)>,
}

fn tree() -> Tree {
    Tree {
        paths: BTreeMap::new(),
        failure: None,
    }
}

impl Tree {
    fn add(mut self, path: &str, directory: bool) -> Self {
        let identity = (1, self.paths.len() as u64 + 1);
        self.paths.insert(
            path.into(),
            Seen {
                canonical: path.into(),
                entry: Entry {
                    directory,
                    identity,
                },
            },
        );
        self
    }

    fn dir(self, path: &str) -> Self {
        self.add(path, true)
    }

    fn file(self, path: &str) -> Self {
        self.add(path, false)
    }

    /// A symlink at `path` to the listed `to`.
    fn link(mut self, path: &str, to: &str) -> Self {
        let entry = self.paths[Path::new(to)].entry;
        self.paths.insert(
            path.into(),
            Seen {
                canonical: to.into(),
                entry,
            },
        );
        self
    }

    /// A second name for the listed path `of`: the same identity and its own
    /// canonical path. It stands for a hard link, and for a case-variant
    /// spelling on a case-insensitive volume.
    fn hard(mut self, path: &str, of: &str) -> Self {
        let entry = self.paths[Path::new(of)].entry;
        self.paths.insert(
            path.into(),
            Seen {
                canonical: path.into(),
                entry,
            },
        );
        self
    }

    fn failing(mut self, ask: Ask, path: &str) -> Self {
        self.failure = Some((ask, path.into()));
        self
    }

    fn seen(&self, ask: Ask, path: &Path) -> Result<&Seen> {
        if self
            .failure
            .as_ref()
            .is_some_and(|(failing, at)| *failing == ask && at == path)
        {
            return Err(Error::from(ErrorKind::PermissionDenied));
        }
        self.paths
            .get(path)
            .ok_or_else(|| Error::from(ErrorKind::NotFound))
    }
}

impl Lookup for Tree {
    fn metadata(&self, path: &Path) -> Result<Entry> {
        self.seen(Ask::Metadata, path).map(|seen| seen.entry)
    }

    fn present(&self, path: &Path) -> Result<()> {
        self.seen(Ask::Present, path).map(|_| ())
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        self.seen(Ask::Canonicalize, path)
            .map(|seen| seen.canonical.clone())
    }

    fn absolute(&self, path: &Path) -> Result<PathBuf> {
        Ok(Path::new("/").join(path))
    }
}

fn project() -> Tree {
    tree()
        .dir("/p")
        .dir("/p/nested")
        .dir("/p/.planning")
        .dir("/p/.planning/phases")
        .dir("/p/.planning/phases/6")
        .file("/p/.planning/state.json")
}

fn linked() -> Tree {
    tree()
        .dir("/p")
        .dir("/p/.planning")
        .file("/p/.planning/config.json")
        .link("/p/alias", "/p/.planning")
        .link("/p/file-alias", "/p/.planning/config.json")
}

#[test]
fn a_cwd_or_target_that_is_empty_or_holds_a_control_byte_is_refused() {
    let fs = tree().dir("/p");
    let cwd = Err(ResolveFailure::BadText(Part::Cwd));
    let target = Err(ResolveFailure::BadText(Part::Target));
    assert_eq!(resolve_target("", "x", &fs), cwd);
    assert_eq!(resolve_target("/p\n", "x", &fs), cwd);
    assert_eq!(resolve_target("/p", "", &fs), target);
    assert_eq!(resolve_target("/p", "bad\0path", &fs), target);
}

#[test]
fn a_relative_cwd_is_refused() {
    assert_eq!(
        resolve_target("p", "x", &tree().dir("p")),
        Err(ResolveFailure::RelativeCwd)
    );
}

#[test]
fn a_cwd_that_is_not_an_existing_directory_is_refused() {
    assert_eq!(
        resolve_target("/p", "x", &tree().file("/p")),
        Err(ResolveFailure::CwdNotDirectory)
    );
    assert!(matches!(
        resolve_target("/p", "x", &tree()),
        Err(ResolveFailure::CwdUnresolved(_))
    ));
}

#[test]
fn a_drive_letter_or_double_slash_target_is_refused() {
    let fs = tree().dir("/p");
    for target in ["C:\\ambiguous\\path", "c:x", "//server/share"] {
        assert_eq!(
            resolve_target("/p", target, &fs),
            Err(ResolveFailure::AmbiguousPrefix),
            "{target}"
        );
    }
}

#[test]
fn dots_resolve_against_the_spelled_path_when_nothing_is_a_link() {
    let fs = project();
    assert_eq!(
        resolve_target("/p", "./.planning/phases/6/../6/SUMMARY.md", &fs),
        Ok("/p/.planning/phases/6/SUMMARY.md".into())
    );
    assert_eq!(
        resolve_target("/p/nested", "../.planning/state.json", &fs),
        Ok("/p/.planning/state.json".into())
    );
}

#[test]
fn a_linked_name_resolves_to_its_target() {
    let fs = linked();
    assert_eq!(
        resolve_target("/p", "alias/config.json", &fs),
        Ok("/p/.planning/config.json".into())
    );
    assert_eq!(
        resolve_target("/p", "file-alias", &fs),
        Ok("/p/.planning/config.json".into())
    );
    assert_eq!(
        resolve_target("/p", "alias/new.md", &fs),
        Ok("/p/.planning/new.md".into())
    );
}

#[test]
fn a_parent_step_after_a_linked_directory_leaves_the_link_target() {
    let fs = tree()
        .dir("/p")
        .dir("/p/.planning")
        .dir("/p/.planning/subdir")
        .file("/p/config.json")
        .link("/p/jump", "/p/.planning/subdir");
    let expected = Ok("/p/.planning/config.json".into());
    assert_eq!(resolve_target("/p", "jump/../config.json", &fs), expected);
    let fs = fs.file("/p/.planning/config.json");
    assert_eq!(resolve_target("/p", "jump/../config.json", &fs), expected);
}

#[test]
fn a_parent_step_above_the_root_is_refused() {
    let escapes = Err(ResolveFailure::AboveRoot);
    assert_eq!(resolve_existing_prefix(Path::new("/.."), &tree()), escapes);
    assert_eq!(resolve_target("/p", "../../x", &tree().dir("/p")), escapes);
}

#[test]
fn a_path_through_a_file_is_refused() {
    let fs = tree().dir("/p").file("/p/notes.md");
    assert_eq!(
        resolve_target("/p", "notes.md/x", &fs),
        Err(ResolveFailure::NonDirectoryParent)
    );
}

#[test]
fn a_lookup_that_fails_other_than_missing_is_refused() {
    let fs = |ask| {
        tree()
            .dir("/p")
            .dir("/p/.planning")
            .failing(ask, "/p/.planning")
    };
    assert!(matches!(
        resolve_target("/p", ".planning/x", &fs(Ask::Metadata)),
        Err(ResolveFailure::Parent(_))
    ));
    assert!(matches!(
        resolve_target("/p", ".planning/x", &fs(Ask::Present)),
        Err(ResolveFailure::Present(_))
    ));
    assert!(matches!(
        resolve_target("/p", ".planning/x", &fs(Ask::Canonicalize)),
        Err(ResolveFailure::Canonicalize(_))
    ));
}

#[test]
fn a_missing_component_is_kept_as_spelled_below_the_existing_prefix() {
    assert_eq!(
        resolve_target("/p", "new/deeper/file.txt", &tree().dir("/p")),
        Ok("/p/new/deeper/file.txt".into())
    );
}

#[test]
fn no_failure_text_names_a_write_or_edit_since_reads_use_it_too() {
    for failure in [
        ResolveFailure::BadText(Part::Cwd),
        ResolveFailure::BadText(Part::Target),
        ResolveFailure::RelativeCwd,
        ResolveFailure::CwdUnresolved("e".into()),
        ResolveFailure::CwdNotDirectory,
        ResolveFailure::AmbiguousPrefix,
        ResolveFailure::AboveRoot,
        ResolveFailure::NonDirectoryParent,
        ResolveFailure::Parent("e".into()),
        ResolveFailure::Present("e".into()),
        ResolveFailure::Canonicalize("e".into()),
        ResolveFailure::Identity("e".into()),
        ResolveFailure::NotAbsolute,
    ] {
        let text = failure.to_string();
        assert!(!text.contains("Write") && !text.contains("Edit"), "{text}");
    }
}

fn inside(path: &str, folder: &str, fs: &Tree) -> bool {
    is_inside(Path::new(path), Path::new(folder), fs).unwrap()
}

fn encloses(path: &str, folder: &str, fs: &Tree) -> bool {
    contains(Path::new(path), Path::new(folder), fs).unwrap()
}

#[test]
fn a_sibling_named_like_the_folder_plus_a_suffix_is_neither_inside_nor_containing_it() {
    let fs = tree()
        .dir("/h")
        .dir("/h/baley")
        .dir("/h/baley-old")
        .file("/h/baley-old/x");
    assert!(!inside("/h/baley-old", "/h/baley", &fs));
    assert!(!inside("/h/baley-old/x", "/h/baley", &fs));
    assert!(!encloses("/h/baley-old", "/h/baley", &fs));
    assert!(inside("/h/baley/x", "/h/baley", &fs));
}

#[test]
fn a_path_equal_to_the_folder_is_inside_it_and_contains_it() {
    let fs = tree().dir("/h").dir("/h/baley");
    assert!(inside("/h/baley", "/h/baley", &fs));
    assert!(encloses("/h/baley", "/h/baley", &fs));
}

#[test]
fn a_path_outside_the_folder_is_not_inside_it() {
    let fs = tree().dir("/h").dir("/h/baley").dir("/elsewhere");
    assert!(!inside("/elsewhere/x", "/h/baley", &fs));
    assert!(!inside("/h", "/h/baley", &fs));
}

#[test]
fn a_case_variant_alias_with_the_folders_identity_is_inside_it_so_identity_is_not_ignored() {
    let fs = tree()
        .dir("/h")
        .dir("/h/baley")
        .hard("/H/Baley", "/h/baley");
    assert!(inside("/H/Baley", "/h/baley", &fs));
    assert!(inside("/H/Baley/keys.env", "/h/baley", &fs));
    assert!(!inside("/H/Other/keys.env", "/h/baley", &fs));
}

#[test]
fn a_parent_given_by_a_case_variant_alias_contains_the_folder() {
    let fs = tree().dir("/h").dir("/h/baley").hard("/H", "/h");
    assert!(encloses("/H", "/h/baley", &fs));
    assert!(!encloses("/H", "/elsewhere/baley", &fs));
}

#[test]
fn a_missing_protected_folder_is_still_matched_by_components() {
    let fs = tree();
    assert!(inside(
        "/h/.config/baley/config.toml",
        "/h/.config/baley",
        &fs
    ));
    assert!(!inside(
        "/h/.config/other/config.toml",
        "/h/.config/baley",
        &fs
    ));
    assert!(encloses("/h/.config", "/h/.config/baley", &fs));
}

#[test]
fn a_missing_protected_folder_is_matched_through_a_case_variant_of_an_existing_parent() {
    let fs = tree().dir("/U/.config").hard("/u/.config", "/U/.config");
    assert!(inside(
        "/u/.config/baley/config.toml",
        "/U/.config/baley",
        &fs
    ));
    assert!(!inside(
        "/u/.config/other/config.toml",
        "/U/.config/baley",
        &fs
    ));
    assert!(!inside("/u/.config", "/U/.config/baley", &fs));
}

#[test]
fn a_case_variant_of_a_missing_protected_folders_own_components_is_inside_it() {
    let fs = tree().dir("/h").dir("/h/.config");
    assert!(inside(
        "/h/.config/Baley/config.toml",
        "/h/.config/baley",
        &fs
    ));
    assert!(!inside(
        "/h/.config/baley-old/config.toml",
        "/h/.config/baley",
        &fs
    ));
}

#[test]
fn a_metadata_lookup_that_fails_other_than_missing_is_a_failure() {
    let broken = |path| {
        tree()
            .dir("/h")
            .dir("/h/baley")
            .failing(Ask::Metadata, path)
    };
    assert!(matches!(
        is_inside(
            Path::new("/h/x"),
            Path::new("/h/baley"),
            &broken("/h/baley")
        ),
        Err(ResolveFailure::Identity(_))
    ));
    assert!(matches!(
        is_inside(Path::new("/h/x"), Path::new("/h/baley"), &broken("/h/x")),
        Err(ResolveFailure::Identity(_))
    ));
    assert!(matches!(
        contains(Path::new("/h/x"), Path::new("/h/baley"), &broken("/h/x")),
        Err(ResolveFailure::Identity(_))
    ));
}
