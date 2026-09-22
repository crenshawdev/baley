use super::{Launch, Output, Process, Recorded, bounded};
use std::io::Cursor;
use std::path::Path;

#[test]
fn a_launch_carries_its_own_environment_including_removals() {
    let launch = Launch::new("git").cwd(Path::new("/project"))
        .args(["status", "--porcelain"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .unset("GIT_LITERAL_PATHSPECS");

    assert_eq!(launch.program, "git");
    assert_eq!(launch.args, ["status", "--porcelain"]);
    assert_eq!(launch.cwd.as_deref(), Some(Path::new("/project")));
    assert_eq!(
        launch.env,
        [
            ("GIT_OPTIONAL_LOCKS".to_owned(), Some("0".into())),
            ("GIT_LITERAL_PATHSPECS".to_owned(), None),
        ]
    );
}

#[test]
fn a_stream_longer_than_the_limit_is_cut_and_says_so() {
    let stream = Cursor::new(vec![b'x'; 9_000]);

    let (bytes, complete) = bounded(stream, 8_192).unwrap();

    assert_eq!(bytes.len(), 8_192);
    assert!(!complete, "a cut stream is incomplete");
}

#[test]
fn a_stream_inside_the_limit_is_whole() {
    let stream = Cursor::new(b"test result: ok. 1 passed\n".to_vec());

    let (bytes, complete) = bounded(stream, 8_192).unwrap();

    assert_eq!(bytes, b"test result: ok. 1 passed\n");
    assert!(complete);
}

#[test]
fn the_fake_answers_in_the_order_it_was_scripted_and_keeps_every_launch() {
    let mut process = Recorded::new().out("main\n").fail(1, "no such ref\n");

    let first = process.run(&Launch::new("git").cwd(Path::new("/a")).arg("branch")).unwrap();
    let second = process.run(&Launch::new("git").cwd(Path::new("/b")).arg("rev-parse")).unwrap();

    assert_eq!(first.stdout, b"main\n");
    assert!(first.success());
    assert_eq!(second.code(), Some(1));
    assert_eq!(second.stderr, b"no such ref\n");
    assert_eq!(process.arguments(), [["branch"], ["rev-parse"]]);
    assert_eq!(process.launches()[1].cwd.as_deref(), Some(Path::new("/b")));
}

#[test]
fn the_fake_can_refuse_to_start_the_program() {
    let mut process =
        Recorded::new().unavailable(std::io::Error::new(std::io::ErrorKind::NotFound, "no git"));

    let answer = process.run(&Launch::new("git").cwd(Path::new("/a")));

    assert_eq!(answer.unwrap_err().kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn a_scripted_exit_reads_back_as_a_code_and_a_signal_reads_back_as_a_signal() {
    assert_eq!(Output::exited(2, "", "").code(), Some(2));
    assert_eq!(Output::exited(2, "", "").signal(), None);
    assert_eq!(Output::signaled(libc::SIGKILL).signal(), Some(libc::SIGKILL));
    assert_eq!(Output::signaled(libc::SIGKILL).code(), None);
}
