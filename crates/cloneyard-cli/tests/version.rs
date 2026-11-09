use std::process::Command;

const EXPECTED: &str = concat!("cloneyard ", env!("CARGO_PKG_VERSION"), "\n");

fn version_of(binary: &str) -> String {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("binary should start");
    assert!(
        output.status.success(),
        "--version exited with {}",
        output.status
    );
    String::from_utf8(output.stdout).expect("stdout should be UTF-8")
}

#[test]
fn cy_prints_the_cloneyard_version() {
    assert_eq!(version_of(env!("CARGO_BIN_EXE_cy")), EXPECTED);
}

#[test]
fn cloneyard_prints_the_cloneyard_version() {
    assert_eq!(version_of(env!("CARGO_BIN_EXE_cloneyard")), EXPECTED);
}
