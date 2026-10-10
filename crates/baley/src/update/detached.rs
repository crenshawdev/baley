//! The server's opt-in update check, started without waiting for its result.

use baley_core::policy::{FileLayer, Schema, SettingsFile, Unavailable, Value, parse_layer};

use crate::process::{Launch, Process};

use super::installation::{HomeRefusal, Layout};

fn should_start(read: Result<Option<SettingsFile>, Unavailable>) -> Result<bool, Unavailable> {
    let Some(file) = read? else {
        return Ok(false);
    };
    let layer = parse_layer(&file, FileLayer::Global, Schema::standard())?;
    Ok(layer.values.iter().any(|setting| {
        setting.host.is_none()
            && setting.name == "updates.auto"
            && setting.value == Value::Bool(true)
    }))
}

fn launch_for(layout: Result<Layout, HomeRefusal>) -> Result<Launch, HomeRefusal> {
    Ok(Launch::new(layout?.stable_path())
        .args(["update", "detached"])
        .own_group()
        .null_output())
}

fn skipped(reason: impl std::fmt::Display) -> Option<String> {
    Some(format!("baley: update check skipped: {reason}").replace(['\r', '\n'], " "))
}

/// Starts an opted-in check once and returns only a line for a refused start.
/// The child keeps running when dropped and is reaped after the server exits.
pub fn start(
    settings: Result<Option<SettingsFile>, Unavailable>,
    layout: Result<Layout, HomeRefusal>,
    process: &mut impl Process,
) -> Option<String> {
    match should_start(settings) {
        Ok(false) => return None,
        Err(error) => return skipped(error),
        Ok(true) => {}
    }
    let launch = match launch_for(layout) {
        Ok(launch) => launch,
        Err(error) => return skipped(error),
    };
    match process.start(&launch) {
        Ok(child) => {
            drop(child);
            None
        }
        Err(error) => skipped(format!(
            "cannot start {}: {error}",
            launch.program.to_string_lossy()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::Environment;
    use crate::process::{Child, Output, StdioPlan, Stream, stdio_plan};
    use crate::settings;
    use baley_core::policy::Fault;
    use std::ffi::{OsStr, OsString};
    use std::io::{self, Read};
    use std::path::Path;
    use std::process::ExitStatus;

    const SETTINGS: &str = "/c/config.toml";

    fn read(bytes: &[u8]) -> Result<Option<SettingsFile>, Unavailable> {
        Ok(Some(settings::file(Path::new(SETTINGS), bytes.to_vec())))
    }

    #[test]
    fn an_update_check_started_without_the_owner_turning_it_on_is_caught() {
        for settings in [
            Ok(None),
            read(b"escalate_on_failure = true\n"),
            read(b"[updates]\nsource = \"https://dl.example/dev\"\n"),
            read(b"[updates]\nauto = false\n"),
        ] {
            assert_eq!(should_start(settings), Ok(false));
        }
        let invalid = should_start(read(b"[")).expect_err("invalid settings cannot opt in");
        assert_eq!(invalid.path, Path::new(SETTINGS));
        let unreadable = Unavailable {
            path: SETTINGS.into(),
            fault: Fault::Unreadable {
                cause: "permission denied".into(),
            },
        };
        assert_eq!(should_start(Err(unreadable.clone())), Err(unreadable));
        assert_eq!(should_start(read(b"[updates]\nauto = true\n")), Ok(true));
    }

    fn layout(home: Option<&str>) -> Result<Layout, HomeRefusal> {
        Layout::resolve(&Environment {
            home: home.map(OsString::from),
            ..Environment::default()
        })
    }

    #[test]
    fn a_detached_check_tied_to_the_server_or_writing_into_its_output_is_caught() {
        let launch = launch_for(layout(Some("/home/o"))).unwrap();
        assert_eq!(launch.program, OsStr::new("/home/o/.local/bin/baley"));
        assert_eq!(launch.args, ["update", "detached"]);
        assert!(launch.own_group);
        assert!(!launch.die_with_parent);
        assert_eq!(launch.timeout, None);
        assert_eq!(launch.stdin, None);
        assert_eq!(
            stdio_plan(&launch),
            StdioPlan {
                stdin: Stream::Null,
                stdout: Stream::Null,
                stderr: Stream::Null,
            }
        );
        assert_eq!(launch_for(layout(None)), Err(HomeRefusal::Unset));
    }

    #[derive(Default)]
    struct Recording {
        starts: Vec<Launch>,
        runs: Vec<Launch>,
    }

    impl Process for Recording {
        fn start(&mut self, launch: &Launch) -> io::Result<Box<dyn Child>> {
            self.starts.push(launch.clone());
            Ok(Box::new(DetachedChild))
        }

        fn run(&mut self, launch: &Launch) -> io::Result<Output> {
            self.runs.push(launch.clone());
            Err(io::Error::other("run waits for the child"))
        }
    }

    struct DetachedChild;

    impl Child for DetachedChild {
        fn stdout(&mut self) -> Option<Box<dyn Read + Send>> {
            None
        }

        fn stderr(&mut self) -> Option<Box<dyn Read + Send>> {
            None
        }

        fn wait(&mut self) -> io::Result<ExitStatus> {
            panic!("the server must not wait for the detached check")
        }

        fn kill(&mut self) -> io::Result<()> {
            panic!("the server must not kill the detached check")
        }
    }

    #[test]
    fn a_detached_check_waited_on_by_the_server_is_caught() {
        let mut process = Recording::default();
        let line = start(
            read(b"[updates]\nauto = true\n"),
            layout(Some("/home/o")),
            &mut process,
        );
        assert!(process.runs.is_empty(), "the server must not call run");
        assert_eq!(process.starts.len(), 1);
        let launch = &process.starts[0];
        assert_eq!(launch.program, OsStr::new("/home/o/.local/bin/baley"));
        assert_eq!(launch.args, ["update", "detached"]);
        assert_eq!(line, None);
    }
}
