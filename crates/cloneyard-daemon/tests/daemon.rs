use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use cloneyard_daemon::system::{self, Version};
use cloneyard_daemon::{Daemon, paths};
use cloneyard_rpc::Client;
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct Dirs {
    temp: TempDir,
    runtime: PathBuf,
    data: PathBuf,
}

impl Dirs {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("run").join("cloneyard");
        let data = temp.path().join("data").join("cloneyard");
        Self {
            temp,
            runtime,
            data,
        }
    }
}

struct Running {
    socket: PathBuf,
    stop: oneshot::Sender<()>,
    task: JoinHandle<anyhow::Result<()>>,
}

async fn start(dirs: &Dirs) -> Running {
    let daemon = Daemon::bind(&dirs.runtime, &dirs.data).await.unwrap();
    let socket = daemon.socket().to_owned();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(daemon.serve(async {
        stopped.await.ok();
    }));
    Running { socket, stop, task }
}

impl Running {
    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap().unwrap();
    }
}

async fn ping(socket: &Path) -> String {
    let (reader, writer) = UnixStream::connect(socket).await.unwrap().into_split();
    Client::new(reader, writer)
        .call(system::PING, ())
        .await
        .unwrap()
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn answers_ping_and_version() {
    let dirs = Dirs::new();
    let daemon = start(&dirs).await;

    let (reader, writer) = UnixStream::connect(&daemon.socket)
        .await
        .unwrap()
        .into_split();
    let mut client = Client::new(reader, writer);
    let pong: String = client.call(system::PING, ()).await.unwrap();
    let version: Version = client.call(system::VERSION, ()).await.unwrap();

    assert_eq!(pong, "pong");
    assert_eq!(version.version, env!("CARGO_PKG_VERSION"));
    daemon.stop().await;
}

#[tokio::test]
async fn answers_malformed_input_with_a_parse_error() {
    let dirs = Dirs::new();
    let daemon = start(&dirs).await;

    let mut stream = UnixStream::connect(&daemon.socket).await.unwrap();
    stream.write_all(b"not json\n").await.unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await.unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();

    assert_eq!(response["error"]["code"], -32700);
    assert_eq!(response["id"], Value::Null);
    daemon.stop().await;
}

#[tokio::test]
async fn creates_the_runtime_dir_with_mode_0700() {
    let dirs = Dirs::new();
    let daemon = start(&dirs).await;

    assert_eq!(mode_of(&dirs.runtime), 0o700);
    daemon.stop().await;
}

#[tokio::test]
async fn sets_an_existing_runtime_dir_to_mode_0700() {
    let dirs = Dirs::new();
    DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&dirs.runtime)
        .unwrap();
    let daemon = start(&dirs).await;

    assert_eq!(mode_of(&dirs.runtime), 0o700);
    daemon.stop().await;
}

#[tokio::test]
async fn creates_the_data_dir_with_mode_0700() {
    let dirs = Dirs::new();
    let daemon = start(&dirs).await;

    assert_eq!(mode_of(&dirs.data), 0o700);
    daemon.stop().await;
}

#[tokio::test]
async fn sets_an_existing_data_dir_to_mode_0700() {
    let dirs = Dirs::new();
    DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&dirs.data)
        .unwrap();
    let daemon = start(&dirs).await;

    assert_eq!(mode_of(&dirs.data), 0o700);
    daemon.stop().await;
}

#[tokio::test]
async fn refuses_a_second_daemon_on_the_same_data_dir() {
    let dirs = Dirs::new();
    let first = start(&dirs).await;
    let other_runtime = dirs.temp.path().join("other").join("cloneyard");

    let Err(error) = Daemon::bind(&other_runtime, &dirs.data).await else {
        panic!("a second daemon opened the same data dir");
    };

    assert_eq!(
        error.to_string(),
        format!("another cloneyard daemon is using {}", dirs.data.display())
    );
    assert_eq!(ping(&first.socket).await, "pong");
    first.stop().await;
}

#[tokio::test]
async fn refuses_a_second_instance() {
    let dirs = Dirs::new();
    let first = start(&dirs).await;

    let Err(error) = Daemon::bind(&dirs.runtime, &dirs.data).await else {
        panic!("a second daemon started");
    };

    assert_eq!(
        error.to_string(),
        format!(
            "cloneyard daemon is already running ({})",
            first.socket.display()
        )
    );
    assert_eq!(ping(&first.socket).await, "pong");
    first.stop().await;
}

#[tokio::test]
async fn removes_the_socket_on_shutdown_and_starts_again() {
    let dirs = Dirs::new();
    let first = start(&dirs).await;
    let socket = first.socket.clone();
    first.stop().await;

    assert!(!socket.exists());
    let second = start(&dirs).await;
    assert_eq!(ping(&second.socket).await, "pong");
    second.stop().await;
}

#[tokio::test]
async fn replaces_a_stale_socket() {
    let dirs = Dirs::new();
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dirs.runtime)
        .unwrap();
    drop(std::os::unix::net::UnixListener::bind(paths::socket(&dirs.runtime)).unwrap());

    let daemon = start(&dirs).await;

    assert_eq!(ping(&daemon.socket).await, "pong");
    daemon.stop().await;
}
