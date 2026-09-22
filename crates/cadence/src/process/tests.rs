use super::{Launch, bounded};
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
