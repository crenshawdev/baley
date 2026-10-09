//! The explicit placement map (D-06): where each artifact goes, as the
//! caller supplies it, and the projections the doctor and the guard take
//! from it (D-07, D-25).
//!
//! The map derives no directory, reads nothing and creates nothing. Where
//! Claude Code keeps skills and settings is a delivery choice `baley
//! install` makes (Build 3 T15) and the doctor observes (T13); both hand
//! the paths in here. An artifact whose place is `Unknown` yields no
//! expected file and no protected path, so an unknown placement can never
//! read as protection already installed.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use super::executable::{Executable, PathFault, judge};
use super::stubs::Entry;

/// An artifact a placement can cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artifact {
    /// The skill stub of one front door, by identity.
    Stub(String),
    /// The MCP registration.
    Registration,
    /// The guard hook.
    Hook,
    /// The security settings, whose content `security::propose` renders.
    Settings,
}

impl fmt::Display for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Artifact::Stub(identity) => write!(f, "stub `{identity}`"),
            Artifact::Registration => f.write_str("registration"),
            Artifact::Hook => f.write_str("hook"),
            Artifact::Settings => f.write_str("settings"),
        }
    }
}

/// Where one artifact goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// A supplied path, judged absolute and UTF-8 when the map is built.
    At(PathBuf),
    /// The caller does not know where the artifact is or should be.
    Unknown,
}

/// Why a placement map cannot be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementRefusal {
    /// A supplied path fails the rule the executable is judged by, so it
    /// cannot reach the protected list as a deny-everything entry or the
    /// expected files as a misleading one.
    Path {
        /// The artifact the path was supplied for.
        artifact: Artifact,
        /// What is wrong with the path.
        fault: PathFault,
    },
    /// The path supplied for the versions folder fails the same rule, so it
    /// cannot reach the protected folders as a deny-everything entry.
    VersionsFolder {
        /// What is wrong with the path.
        fault: PathFault,
    },
    /// A stub's file would hold something else too. The doctor checks a
    /// stub byte for byte, so its file holds only that stub.
    StubFileShared {
        /// The stub's identity.
        stub: String,
        /// The other artifact placed at the same path.
        with: Artifact,
        /// The shared path.
        path: PathBuf,
    },
    /// One identity was given two stub placements.
    DuplicateStub(String),
    /// A stub's file is the executable the hook and the registration run
    /// (D-25), so placing the stub would replace the binary.
    StubOverExecutable {
        /// The stub's identity.
        stub: String,
        /// The executable's path as supplied.
        executable: PathBuf,
    },
}

impl fmt::Display for PlacementRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlacementRefusal::Path { artifact, fault } => {
                write!(f, "the path supplied for the {artifact} {fault}")
            }
            PlacementRefusal::VersionsFolder { fault } => {
                write!(f, "the path supplied for the versions folder {fault}")
            }
            PlacementRefusal::StubFileShared { stub, with, path } => write!(
                f,
                "stub `{stub}` cannot share {} with the {with}",
                path.display()
            ),
            PlacementRefusal::DuplicateStub(identity) => {
                write!(f, "stub `{identity}` is placed twice")
            }
            PlacementRefusal::StubOverExecutable { stub, executable } => write!(
                f,
                "stub `{stub}` cannot be placed over the executable {}",
                executable.display()
            ),
        }
    }
}

/// One placement per artifact and the judged executable. Two JSON artifacts
/// (the registration, the hook and the settings) may name one file, as the
/// hook and the settings do when `baley install` writes both into one
/// settings document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementMap {
    executable: Executable,
    stubs: Vec<(Entry, Placement)>,
    registration: Placement,
    hook: Placement,
    settings: Placement,
    versions: Placement,
}

/// A file the doctor expects on disk: the artifact, its path and, for a
/// stub, the manifest entry whose bytes and digest the file must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedFile<'a> {
    /// The artifact placed there.
    pub artifact: Artifact,
    /// The supplied path.
    pub path: &'a Path,
    /// The stub's manifest entry, so a placed file can be compared without
    /// rebuilding the manifest. `None` for a JSON artifact.
    pub stub: Option<&'a Entry>,
}

/// One registration or settings file and every JSON artifact that lands in
/// it, so composition runs once per document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document<'a> {
    /// The supplied path.
    pub path: &'a Path,
    /// The artifacts placed in it, in artifact order.
    pub artifacts: Vec<Artifact>,
}

impl PlacementMap {
    /// Builds the map, judging every supplied path, the versions folder's
    /// included, by the executable's rule and refusing a stub whose identity
    /// is placed twice or whose file is another artifact's file or the
    /// executable. The versions folder is not an artifact: it adds no expected
    /// file and no protected path, only [`PlacementMap::write_only_folders`].
    ///
    /// Files are compared by spelling with `.` and `..` resolved and nothing
    /// read, so two spellings joined by a symbolic link are not seen here.
    pub fn new(
        executable: Executable,
        stubs: Vec<(Entry, Placement)>,
        registration: Placement,
        hook: Placement,
        settings: Placement,
        versions: Placement,
    ) -> Result<Self, PlacementRefusal> {
        let map = Self {
            executable,
            stubs,
            registration,
            hook,
            settings,
            versions,
        };
        if let Placement::At(path) = &map.versions
            && let Err(fault) = judge(path.as_os_str())
        {
            return Err(PlacementRefusal::VersionsFolder { fault });
        }
        for (artifact, placement, _) in map.all() {
            if let Placement::At(path) = placement
                && let Err(fault) = judge(path.as_os_str())
            {
                return Err(PlacementRefusal::Path { artifact, fault });
            }
        }
        for (index, (entry, placement)) in map.stubs.iter().enumerate() {
            if map.stubs[..index]
                .iter()
                .any(|(earlier, _)| earlier.identity == entry.identity)
            {
                return Err(PlacementRefusal::DuplicateStub(entry.identity.clone()));
            }
            let Placement::At(path) = placement else {
                continue;
            };
            let stub = Artifact::Stub(entry.identity.clone());
            let file = resolved(path);
            let shared = map.all().into_iter().find(|(artifact, other, _)| {
                *artifact != stub
                    && matches!(other, Placement::At(other) if resolved(other) == file)
            });
            if let Some((with, _, _)) = shared {
                return Err(PlacementRefusal::StubFileShared {
                    stub: entry.identity.clone(),
                    with,
                    path: path.clone(),
                });
            }
            let executable = Path::new(map.executable.as_str());
            if resolved(executable) == file {
                return Err(PlacementRefusal::StubOverExecutable {
                    stub: entry.identity.clone(),
                    executable: executable.to_path_buf(),
                });
            }
        }
        Ok(map)
    }

    /// The judged executable the hook and the registration run.
    pub fn executable(&self) -> &Executable {
        &self.executable
    }

    /// Every artifact with its placement, in artifact order: the stubs in
    /// manifest order, then the registration, the hook and the settings.
    fn all(&self) -> Vec<(Artifact, &Placement, Option<&Entry>)> {
        let mut all: Vec<_> = self
            .stubs
            .iter()
            .map(|(entry, placement)| {
                (
                    Artifact::Stub(entry.identity.clone()),
                    placement,
                    Some(entry),
                )
            })
            .collect();
        all.push((Artifact::Registration, &self.registration, None));
        all.push((Artifact::Hook, &self.hook, None));
        all.push((Artifact::Settings, &self.settings, None));
        all
    }

    /// The files the doctor expects: one per artifact with a known place.
    pub fn expected_files(&self) -> Vec<ExpectedFile<'_>> {
        self.all()
            .into_iter()
            .filter_map(|(artifact, placement, stub)| match placement {
                Placement::At(path) => Some(ExpectedFile {
                    artifact,
                    path,
                    stub,
                }),
                Placement::Unknown => None,
            })
            .collect()
    }

    /// The distinct known files among the registration, the hook and the
    /// settings, each once with the artifacts that land in it.
    pub fn documents(&self) -> Vec<Document<'_>> {
        let mut documents: Vec<Document<'_>> = Vec::new();
        for (artifact, placement, _) in self.all() {
            if matches!(artifact, Artifact::Stub(_)) {
                continue;
            }
            let Placement::At(path) = placement else {
                continue;
            };
            match documents
                .iter_mut()
                .find(|document| document.path == path.as_path())
            {
                Some(document) => document.artifacts.push(artifact),
                None => documents.push(Document {
                    path,
                    artifacts: vec![artifact],
                }),
            }
        }
        documents
    }

    /// The paths the guard protects from a Write, Edit or NotebookEdit
    /// (D-07, D-25): the distinct known paths of the stubs, the
    /// registration, the hook and the settings in first-seen order, each
    /// file once, then the executable.
    ///
    /// This is the list T15 appends to `ProtectedPaths.files` at
    /// `guard_hook::context::judge`. Until then, production keeps that list
    /// to the session project's `baley.toml` and the cwd checkout's
    /// `baley.toml` (D-08). Those files are never in this list: the guard
    /// finds them at runtime, and the map takes no settings-file input, so
    /// the exact-list test on a full map is what keeps them out.
    ///
    /// The versions folder is not in this list, because a file entry would
    /// protect only the folder's own name and leave the binaries inside it
    /// writable. T15 passes [`PlacementMap::write_only_folders`] beside this
    /// list, to the proposal and to the guard's protected paths.
    pub fn protected_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut add = |path: PathBuf| {
            if !paths.contains(&path) {
                paths.push(path);
            }
        };
        for (_, placement, _) in self.all() {
            if let Placement::At(path) = placement {
                add(path.clone());
            }
        }
        add(PathBuf::from(self.executable.as_str()));
        paths
    }

    /// The folders the guard and the sandbox deny writes into, and never
    /// reads: the versions folder when it is placed, and nothing when it is
    /// unknown, so an unknown place never reads as protection installed.
    pub fn write_only_folders(&self) -> Vec<PathBuf> {
        match &self.versions {
            Placement::At(path) => vec![path.clone()],
            Placement::Unknown => Vec::new(),
        }
    }

    /// Every artifact whose place is unknown, by name.
    pub fn not_installed(&self) -> Vec<Artifact> {
        self.all()
            .into_iter()
            .filter(|(_, placement, _)| **placement == Placement::Unknown)
            .map(|(artifact, _, _)| artifact)
            .collect()
    }
}

/// An absolute path's components with `.` dropped and `..` popping its
/// parent, from the text alone.
fn resolved(path: &Path) -> Vec<Component<'_>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            // `..` at the root stays at the root.
            Component::ParentDir => {
                if matches!(parts.last(), Some(Component::Normal(_))) {
                    parts.pop();
                }
            }
            other => parts.push(other),
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_artifacts::stubs::{front_doors, manifest};

    const EXECUTABLE: &str = "/usr/local/bin/baley";
    const CAPTURE: &str = "/u/.claude/skills/bal-capture/SKILL.md";
    const HELP: &str = "/u/.claude/skills/bal-help/SKILL.md";
    const REGISTRATION: &str = "/u/.claude.json";
    const HOOK: &str = "/u/.claude/hooks.json";
    const SETTINGS: &str = "/u/.claude/settings.json";

    fn at(path: &str) -> Placement {
        Placement::At(path.into())
    }

    fn stubs(capture: Placement, help: Placement) -> Vec<(Entry, Placement)> {
        let entries = manifest(&front_doors()).unwrap();
        assert_eq!(entries[0].identity, "bal-capture");
        assert_eq!(entries[1].identity, "bal-help");
        entries.into_iter().zip([capture, help]).collect()
    }

    fn build(
        stubs: Vec<(Entry, Placement)>,
        registration: Placement,
        hook: Placement,
        settings: Placement,
    ) -> Result<PlacementMap, PlacementRefusal> {
        PlacementMap::new(
            Executable::new(EXECUTABLE).unwrap(),
            stubs,
            registration,
            hook,
            settings,
            Placement::Unknown,
        )
    }

    fn build_with_versions(versions: Placement) -> Result<PlacementMap, PlacementRefusal> {
        PlacementMap::new(
            Executable::new(EXECUTABLE).unwrap(),
            stubs(Placement::Unknown, Placement::Unknown),
            Placement::Unknown,
            Placement::Unknown,
            Placement::Unknown,
            versions,
        )
    }

    #[test]
    fn a_placed_path_left_out_of_protection_or_the_executable_omitted_is_caught() {
        let map = build(
            stubs(at(CAPTURE), at(HELP)),
            at(REGISTRATION),
            at(HOOK),
            at(SETTINGS),
        )
        .unwrap();
        let files = map.expected_files();
        let listed: Vec<(&Artifact, &Path)> = files
            .iter()
            .map(|file| (&file.artifact, file.path))
            .collect();
        assert_eq!(
            listed,
            [
                (&Artifact::Stub("bal-capture".into()), Path::new(CAPTURE)),
                (&Artifact::Stub("bal-help".into()), Path::new(HELP)),
                (&Artifact::Registration, Path::new(REGISTRATION)),
                (&Artifact::Hook, Path::new(HOOK)),
                (&Artifact::Settings, Path::new(SETTINGS)),
            ]
        );
        for file in &files {
            match &file.artifact {
                Artifact::Stub(identity) => {
                    let entry = file.stub.expect("a stub carries its entry");
                    assert_eq!(&entry.identity, identity);
                    assert!(!entry.bytes.is_empty() && entry.digest.len() == 64);
                }
                _ => assert_eq!(file.stub, None),
            }
        }
        let expected: Vec<PathBuf> = [CAPTURE, HELP, REGISTRATION, HOOK, SETTINGS, EXECUTABLE]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(map.protected_paths(), expected);
        assert!(map.not_installed().is_empty());
    }

    #[test]
    fn a_shared_document_listed_twice_or_composed_twice_is_caught() {
        let map = build(
            stubs(at(CAPTURE), at(HELP)),
            at(REGISTRATION),
            at(SETTINGS),
            at(SETTINGS),
        )
        .unwrap();
        assert_eq!(
            map.documents(),
            [
                Document {
                    path: Path::new(REGISTRATION),
                    artifacts: vec![Artifact::Registration],
                },
                Document {
                    path: Path::new(SETTINGS),
                    artifacts: vec![Artifact::Hook, Artifact::Settings],
                },
            ]
        );
        let protected = map.protected_paths();
        assert_eq!(
            protected
                .iter()
                .filter(|path| path.as_path() == Path::new(SETTINGS))
                .count(),
            1
        );
        assert_eq!(protected.len(), 5);
        assert_eq!(map.expected_files().len(), 5);
    }

    #[test]
    fn an_unknown_placement_read_as_installed_or_the_executable_dropped_with_nothing_placed_is_caught()
     {
        let map = build(
            stubs(Placement::Unknown, Placement::Unknown),
            Placement::Unknown,
            Placement::Unknown,
            Placement::Unknown,
        )
        .unwrap();
        assert!(map.expected_files().is_empty());
        assert!(map.documents().is_empty());
        assert_eq!(
            map.not_installed(),
            [
                Artifact::Stub("bal-capture".into()),
                Artifact::Stub("bal-help".into()),
                Artifact::Registration,
                Artifact::Hook,
                Artifact::Settings,
            ]
        );
        assert_eq!(map.protected_paths(), [PathBuf::from(EXECUTABLE)]);
    }

    #[test]
    fn a_relative_placement_or_two_stubs_on_one_file_accepted_is_caught() {
        let relative_stub = build(
            stubs(at(CAPTURE), at("skills/bal-help/SKILL.md")),
            at(REGISTRATION),
            at(HOOK),
            at(SETTINGS),
        );
        assert_eq!(
            relative_stub,
            Err(PlacementRefusal::Path {
                artifact: Artifact::Stub("bal-help".into()),
                fault: PathFault::Relative,
            })
        );
        let relative_hook = build(
            stubs(at(CAPTURE), at(HELP)),
            at(REGISTRATION),
            at("hooks.json"),
            Placement::Unknown,
        );
        assert_eq!(
            relative_hook,
            Err(PlacementRefusal::Path {
                artifact: Artifact::Hook,
                fault: PathFault::Relative,
            })
        );
        let two_stubs = build(
            stubs(at(HELP), at(HELP)),
            at(REGISTRATION),
            at(HOOK),
            at(SETTINGS),
        );
        assert_eq!(
            two_stubs,
            Err(PlacementRefusal::StubFileShared {
                stub: "bal-capture".into(),
                with: Artifact::Stub("bal-help".into()),
                path: HELP.into(),
            })
        );
        let stub_in_hook = build(
            stubs(at(CAPTURE), at(HOOK)),
            at(REGISTRATION),
            at(HOOK),
            at(SETTINGS),
        );
        assert_eq!(
            stub_in_hook,
            Err(PlacementRefusal::StubFileShared {
                stub: "bal-help".into(),
                with: Artifact::Hook,
                path: HOOK.into(),
            })
        );
    }

    #[test]
    fn a_stub_sharing_a_file_under_another_spelling_accepted_is_caught() {
        let help_in_hook = build(
            stubs(at(CAPTURE), at("/p/SKILL.md")),
            at(REGISTRATION),
            at("/p/d/../SKILL.md"),
            at(SETTINGS),
        );
        assert_eq!(
            help_in_hook,
            Err(PlacementRefusal::StubFileShared {
                stub: "bal-help".into(),
                with: Artifact::Hook,
                path: "/p/SKILL.md".into(),
            })
        );
        let two_stubs = build(
            stubs(at("/p/x/../SKILL.md"), at("/p/SKILL.md")),
            at(REGISTRATION),
            at(HOOK),
            at(SETTINGS),
        );
        assert_eq!(
            two_stubs,
            Err(PlacementRefusal::StubFileShared {
                stub: "bal-capture".into(),
                with: Artifact::Stub("bal-help".into()),
                path: "/p/x/../SKILL.md".into(),
            })
        );
        let apart = build(
            stubs(at(CAPTURE), at("/p/SKILL.md")),
            at(REGISTRATION),
            at("/p/d/SKILL.md"),
            at(SETTINGS),
        );
        assert!(apart.is_ok(), "{apart:?}");
    }

    #[test]
    fn a_stub_placed_over_the_executable_accepted_is_caught() {
        for (executable, help) in [
            (EXECUTABLE, EXECUTABLE),
            (EXECUTABLE, "/usr/local/lib/../bin/baley"),
            ("/usr/local/x/../bin/baley", EXECUTABLE),
        ] {
            let map = PlacementMap::new(
                Executable::new(executable).unwrap(),
                stubs(at(CAPTURE), at(help)),
                at(REGISTRATION),
                at(HOOK),
                at(SETTINGS),
                Placement::Unknown,
            );
            assert_eq!(
                map,
                Err(PlacementRefusal::StubOverExecutable {
                    stub: "bal-help".into(),
                    executable: executable.into(),
                }),
                "{executable} {help}"
            );
        }
    }

    #[test]
    fn an_unknown_versions_folder_projected_as_protection_or_a_relative_one_accepted_is_caught() {
        const VERSIONS: &str = "/home/o/.local/lib/crenshawdev/baley/versions";
        let unknown = build_with_versions(Placement::Unknown).unwrap();
        assert_eq!(unknown.write_only_folders(), Vec::<PathBuf>::new());
        assert_eq!(unknown.protected_paths(), [PathBuf::from(EXECUTABLE)]);

        let placed = build_with_versions(at(VERSIONS)).unwrap();
        assert_eq!(placed.write_only_folders(), [PathBuf::from(VERSIONS)]);
        assert_eq!(placed.protected_paths(), [PathBuf::from(EXECUTABLE)]);
        assert_eq!(placed.not_installed().len(), 5);
        assert_eq!(placed.not_installed(), unknown.not_installed());

        let relative = build_with_versions(at("lib/versions")).expect_err("a relative folder");
        assert_eq!(
            relative,
            PlacementRefusal::VersionsFolder {
                fault: PathFault::Relative
            }
        );
        let text = relative.to_string();
        assert!(text.contains("versions folder"), "{text}");
        assert!(text.contains("not absolute"), "{text}");
    }
}
