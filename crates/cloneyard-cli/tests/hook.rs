use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use common::Node;

mod common;

fn hook(node: &Node, event: &str, spool: Option<&Path>, payload: &str) -> Output {
    let mut cy = node.cy("hook");
    cy.arg(event)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(spool) = spool {
        cy.env("CLONEYARD_SPOOL", spool);
    }
    let mut child = cy.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
}

fn quiet_success(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn spool(node: &Node) -> PathBuf {
    let spool = node.home.path().join("spool");
    fs::create_dir(&spool).unwrap();
    spool
}

#[test]
fn writes_stdin_into_the_spool_as_one_file_named_by_time_pid_and_event_and_prints_nothing() {
    let node = Node::new();
    let spool = spool(&node);

    let output = hook(
        &node,
        "PreToolUse",
        Some(&spool),
        r#"{"session_id":"2f6c"}"#,
    );

    assert_eq!(quiet_success(&output), "");
    let [file] = files(&spool).try_into().unwrap();
    let name = file.file_name().unwrap().to_str().unwrap();
    let parts: Vec<&str> = name.splitn(3, '-').collect();
    assert_eq!(parts.len(), 3, "{name}");
    assert_eq!(parts[0].len(), 20, "{name}");
    assert!(parts[0].bytes().all(|byte| byte.is_ascii_digit()), "{name}");
    assert!(parts[1].parse::<u32>().is_ok(), "{name}");
    assert_eq!(parts[2], "PreToolUse.json");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        r#"{"session_id":"2f6c"}"#
    );
}

#[test]
fn a_later_event_sorts_after_an_earlier_one() {
    let node = Node::new();
    let spool = spool(&node);

    for event in ["UserPromptSubmit", "PreToolUse", "Stop"] {
        quiet_success(&hook(&node, event, Some(&spool), "{}"));
    }

    let events: Vec<String> = files(&spool)
        .iter()
        .map(|file| {
            let name = file.file_stem().unwrap().to_str().unwrap();
            name.rsplit('-').next().unwrap().to_owned()
        })
        .collect();
    assert_eq!(events, ["UserPromptSubmit", "PreToolUse", "Stop"]);
}

#[test]
fn exits_zero_and_says_why_on_stderr_without_a_spool_or_with_one_it_cannot_write() {
    let node = Node::new();
    let missing = node.home.path().join("missing");
    let read_only = spool(&node);
    fs::set_permissions(&read_only, fs::Permissions::from_mode(0o500)).unwrap();

    let unset = quiet_success(&hook(&node, "Stop", None, "{}"));
    let errors: Vec<String> = [&missing, &read_only]
        .map(|dir| quiet_success(&hook(&node, "Stop", Some(dir), "{}")))
        .into();
    let invalid = quiet_success(&hook(&node, "../Stop", Some(&read_only), "{}"));

    assert_eq!(unset, "cy hook Stop: CLONEYARD_SPOOL is not set\n");
    for (dir, error, reason) in [
        (
            &missing,
            &errors[0],
            "No such file or directory (os error 2)",
        ),
        (&read_only, &errors[1], "Permission denied (os error 13)"),
    ] {
        let prefix = format!("cy hook Stop: cannot create {}/.", dir.display());
        assert!(error.starts_with(&prefix), "{error}");
        assert!(
            error.ends_with(&format!("-Stop.tmp: {reason}\n")),
            "{error}"
        );
    }
    assert_eq!(invalid, "cy hook ../Stop: invalid hook name \"../Stop\"\n");
    assert_eq!(files(&read_only), Vec::<PathBuf>::new());
}
