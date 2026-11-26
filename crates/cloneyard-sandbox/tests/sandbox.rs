//! Each test enters the sandbox in a child process, this test binary run again, since a process can't leave its sandbox.

use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::symlink;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use cloneyard_sandbox::Policy;
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::net::{AddressFamily, SocketType, socket};
use tempfile::TempDir;

const POLICY: &str = "CLONEYARD_TEST_POLICY";
const PROBE: &str = "CLONEYARD_TEST_PROBE";
const LEAK: &str = "CLONEYARD_TEST_LEAK";
const PROGRAM: &str = "CLONEYARD_TEST_PROGRAM";

/// In the child, enters the sandbox in `POLICY` and runs `probe` in it; it does nothing in a normal run.
#[test]
fn enter() {
    let Some(policy) = env::var_os(POLICY) else {
        return;
    };
    let leaked = env::var_os(LEAK).map(|path| {
        let file = File::open(path).unwrap();
        fcntl_setfd(&file, FdFlags::empty()).unwrap();
        file
    });
    let args: Vec<OsString> = ["--exact", "probe", "--nocapture", "--quiet"]
        .map(OsString::from)
        .into();
    let program = env::var_os(PROGRAM).unwrap_or_else(|| env::current_exe().unwrap().into());
    let error = cloneyard_sandbox::enter(policy.to_str().unwrap(), &program, &args);
    drop(leaked);
    println!("result: {error}");
}

/// In the sandboxed child, does what `PROBE` says, such as `read <path>`, and prints how it went.
#[test]
fn probe() {
    let Some(probe) = env::var_os(PROBE) else {
        return;
    };
    let probe = probe.into_string().unwrap();
    let (what, path) = probe.split_once(' ').unwrap_or((probe.as_str(), ""));
    let result = match what {
        "read" => fs::read(path).map(drop),
        "list" => fs::read_dir(path).map(drop),
        "write" => fs::write(path, "x"),
        "unix-stream" => UnixStream::connect(path).map(drop),
        "unix-seqpacket" => socket(AddressFamily::UNIX, SocketType::SEQPACKET, None)
            .map(drop)
            .map_err(Into::into),
        "unix-datagram" => UnixDatagram::unbound().map(drop),
        "socketpair" => UnixStream::pair().map(drop),
        "io-uring" => io_uring::IoUring::new(8).map(drop),
        "fds" => {
            println!("result: {}", open_fds().join(" "));
            return;
        }
        "wait" => {
            println!("result: waiting");
            io::stdout().flush().unwrap();
            io::stdin().read_to_end(&mut Vec::new()).map(drop)
        }
        _ => panic!("no probe {what}"),
    };
    match result {
        Ok(()) => println!("result: ok"),
        Err(error) => println!("result: {error}"),
    }
}

/// The descriptors open in this process, without the one that lists them.
fn open_fds() -> Vec<String> {
    let listed: Vec<String> = fs::read_dir("/proc/self/fd")
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    listed
        .into_iter()
        .filter(|fd| fs::symlink_metadata(PathBuf::from("/proc/self/fd").join(fd)).is_ok())
        .collect()
}

/// What `probe` printed after entering a sandbox with `policy`.
fn sandboxed(policy: &Policy, probe: &str) -> String {
    sandboxed_with(policy, probe, |_| {})
}

fn sandboxed_with(policy: &Policy, probe: &str, setup: impl FnOnce(&mut Command)) -> String {
    let mut child = Command::new(env::current_exe().unwrap());
    child
        .args(["--exact", "enter", "--nocapture", "--quiet"])
        .env(POLICY, serde_json::to_string(policy).unwrap())
        .env(PROBE, probe);
    setup(&mut child);
    let output = child.output().unwrap();
    result(&output)
}

fn result(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("result: "))
        .unwrap_or_else(|| panic!("no result in {output:?}"))
        .to_owned()
}

fn policy(hidden: &[PathBuf], visible: &[PathBuf], writable: &[PathBuf]) -> Policy {
    Policy::new(hidden.to_vec(), visible.to_vec(), writable.to_vec()).unwrap()
}

/// A temporary directory, given as its canonical path, with the directory itself kept alive.
fn dir() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    (dir, path)
}

const DENIED: &str = "Permission denied (os error 13)";

#[test]
fn a_hidden_directory_and_what_it_holds_cannot_be_read_while_its_siblings_and_parent_can() {
    let (_dir, dir) = dir();
    fs::create_dir(dir.join("secret")).unwrap();
    fs::write(dir.join("secret/key"), "key").unwrap();
    fs::write(dir.join("open"), "open").unwrap();
    let policy = policy(&[dir.join("secret")], &[], &[]);

    let key = sandboxed(
        &policy,
        &format!("read {}", dir.join("secret/key").display()),
    );
    let open = sandboxed(&policy, &format!("read {}", dir.join("open").display()));
    let parent = sandboxed(&policy, &format!("list {}", dir.display()));

    assert_eq!(
        (key.as_str(), open.as_str(), parent.as_str()),
        (DENIED, "ok", "ok")
    );
}

#[test]
fn a_hidden_file_cannot_be_read_while_the_file_next_to_it_can() {
    let (_dir, dir) = dir();
    fs::write(dir.join(".env"), "TOKEN=1").unwrap();
    fs::write(dir.join("README.md"), "app").unwrap();
    let policy = policy(&[dir.join(".env")], &[], &[]);

    let env = sandboxed(&policy, &format!("read {}", dir.join(".env").display()));
    let readme = sandboxed(
        &policy,
        &format!("read {}", dir.join("README.md").display()),
    );

    assert_eq!((env.as_str(), readme.as_str()), (DENIED, "ok"));
}

#[test]
fn a_visible_path_inside_a_hidden_one_can_be_read_and_nothing_else_there() {
    let (_dir, dir) = dir();
    fs::create_dir_all(dir.join("repo/.git/objects")).unwrap();
    fs::write(dir.join("repo/.git/objects/pack"), "pack").unwrap();
    fs::write(dir.join("repo/.env"), "TOKEN=1").unwrap();
    let policy = policy(&[dir.join("repo")], &[dir.join("repo/.git/objects")], &[]);

    let objects = sandboxed(
        &policy,
        &format!("read {}", dir.join("repo/.git/objects/pack").display()),
    );
    let env = sandboxed(
        &policy,
        &format!("read {}", dir.join("repo/.env").display()),
    );

    assert_eq!((objects.as_str(), env.as_str()), ("ok", DENIED));
}

#[test]
fn a_symlink_to_a_hidden_path_does_not_open_it() {
    let (_dir, dir) = dir();
    fs::create_dir(dir.join("secret")).unwrap();
    fs::write(dir.join("secret/key"), "key").unwrap();
    symlink(dir.join("secret"), dir.join("link")).unwrap();
    let policy = policy(&[dir.join("secret")], &[], &[]);

    let through_link = sandboxed(&policy, &format!("read {}", dir.join("link/key").display()));

    assert_eq!(through_link, DENIED);
}

#[test]
fn a_clone_writes_in_its_writable_paths_and_nowhere_else() {
    let (_dir, dir) = dir();
    fs::create_dir(dir.join("work")).unwrap();
    let policy = policy(&[], &[], &[dir.join("work")]);

    let inside = sandboxed(
        &policy,
        &format!("write {}", dir.join("work/file").display()),
    );
    let outside = sandboxed(&policy, &format!("write {}", dir.join("file").display()));

    assert_eq!((inside.as_str(), outside.as_str()), ("ok", DENIED));
}

#[test]
fn unix_stream_and_seqpacket_sockets_fail_while_datagram_sockets_and_socketpairs_work() {
    let (_dir, dir) = dir();
    let path = dir.join("daemon.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    let policy = policy(&[], &[], &[]);

    let results = [
        format!("unix-stream {}", path.display()),
        "unix-seqpacket".to_owned(),
        "unix-datagram".to_owned(),
        "socketpair".to_owned(),
    ]
    .map(|probe| sandboxed(&policy, &probe));

    assert_eq!(results, [DENIED, DENIED, "ok", "ok"]);
}

#[test]
fn io_uring_fails_with_eperm() {
    let policy = policy(&[], &[], &[]);

    let result = sandboxed(&policy, "io-uring");

    assert_eq!(result, "Operation not permitted (os error 1)");
}

#[test]
fn a_descriptor_the_sandbox_inherited_does_not_reach_the_program() {
    let (_dir, dir) = dir();
    fs::write(dir.join("leaked"), "").unwrap();
    let policy = policy(&[], &[], &[]);

    let result = sandboxed_with(&policy, "fds", |child| {
        child.env(LEAK, dir.join("leaked"));
    });

    assert_eq!(result, "0 1 2");
}

#[test]
fn a_sandboxed_process_cannot_read_the_environment_of_one_in_another_sandbox() {
    let policy = policy(&[], &[], &[]);
    let mut sibling = Command::new(env::current_exe().unwrap())
        .args(["--exact", "enter", "--nocapture", "--quiet"])
        .env(POLICY, serde_json::to_string(&policy).unwrap())
        .env(PROBE, "wait")
        .env("CLONEYARD_TEST_SECRET", "token-of-the-sibling")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(sibling.stdout.take().unwrap())
        .lines()
        .map(Result::unwrap);
    let waiting = lines.find(|line| line.starts_with("result: "));
    let environ = format!("/proc/{}/environ", sibling.id());

    let outside = fs::read(&environ).unwrap();
    let inside = sandboxed(&policy, &format!("read {environ}"));

    drop(sibling.stdin.take());
    lines.for_each(drop);
    sibling.wait().unwrap();
    assert_eq!(waiting.as_deref(), Some("result: waiting"));
    assert!(
        String::from_utf8_lossy(&outside).contains("CLONEYARD_TEST_SECRET=token-of-the-sibling")
    );
    assert_eq!(inside, DENIED);
}

#[test]
fn a_writable_path_that_is_missing_names_the_path() {
    let (_dir, dir) = dir();
    let policy = policy(&[], &[], &[dir.join("missing")]);

    let result = sandboxed(&policy, "read /dev/null");

    assert_eq!(
        result,
        format!(
            "cannot open {}: No such file or directory (os error 2)",
            dir.join("missing").display()
        )
    );
}

#[test]
fn a_program_that_cannot_run_is_named() {
    let (_dir, dir) = dir();
    let policy = policy(&[], &[], &[]);

    let result = sandboxed_with(&policy, "read /dev/null", |child| {
        child.env(PROGRAM, dir.join("claude"));
    });

    assert_eq!(
        result,
        format!(
            "cannot run {}: No such file or directory (os error 2)",
            dir.join("claude").display()
        )
    );
}
