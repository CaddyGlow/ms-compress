#![cfg(feature = "cli")]
use std::{
    fs,
    io::Write,
    process::{Command, Output, Stdio},
};

fn run(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ms-compress"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn stdin_stdout_roundtrips_all_cli_formats() {
    let input = b"CLI buffer compatibility test ".repeat(100);
    for format in ["lznt1", "xpress", "xpress-huffman"] {
        let encoded = run(&["rtlcompress", "--format", format], &input);
        assert!(encoded.status.success(), "{:?}", encoded.stderr);
        assert!(encoded.stderr.is_empty());
        let decoded = run(
            &[
                "rtldecompress",
                "--format",
                format,
                "--output-size",
                &input.len().to_string(),
            ],
            &encoded.stdout,
        );
        assert!(decoded.status.success(), "{:?}", decoded.stderr);
        assert_eq!(decoded.stdout, input);
        assert!(decoded.stderr.is_empty());
    }
    for format in ["lznt1", "xpress"] {
        let encoded = run(&["rtlcompress", "--format", format], &[]);
        assert!(encoded.status.success());
        let decoded = run(
            &["rtldecompress", "--format", format, "--output-size", "0"],
            &encoded.stdout,
        );
        assert!(decoded.status.success());
        assert!(decoded.stdout.is_empty());
    }
}

#[test]
fn file_outputs_are_atomic_and_existing_files_require_force() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input");
    let encoded = directory.path().join("encoded");
    let decoded = directory.path().join("decoded");
    fs::write(&input, b"abcabcabc").unwrap();
    let result = run(
        &[
            "rtlcompress",
            "-i",
            input.to_str().unwrap(),
            "-o",
            encoded.to_str().unwrap(),
        ],
        &[],
    );
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    let result = run(
        &[
            "rtldecompress",
            "-i",
            encoded.to_str().unwrap(),
            "-o",
            decoded.to_str().unwrap(),
            "--output-size",
            "9",
        ],
        &[],
    );
    assert!(result.status.success());
    assert_eq!(fs::read(&decoded).unwrap(), b"abcabcabc");
    fs::write(&decoded, b"preserved").unwrap();
    let result = run(
        &["rtlcompress", "-o", decoded.to_str().unwrap()],
        b"replacement",
    );
    assert!(!result.status.success());
    assert_eq!(fs::read(&decoded).unwrap(), b"preserved");
    let result = run(
        &["rtlcompress", "-o", decoded.to_str().unwrap(), "--force"],
        b"replacement",
    );
    assert!(result.status.success());
    assert_ne!(fs::read(&decoded).unwrap(), b"preserved");
    // Even --force never replaces a destination after a decode error.
    let before = fs::read(&decoded).unwrap();
    let result = run(
        &[
            "rtldecompress",
            "-o",
            decoded.to_str().unwrap(),
            "--force",
            "--output-size",
            "9",
        ],
        &[1],
    );
    assert!(!result.status.success());
    assert_eq!(fs::read(&decoded).unwrap(), before);
}

#[test]
fn malformed_inputs_sizes_and_resource_limits_fail_without_output() {
    for (args, input) in [
        (vec!["rtlcompress", "--format", "unknown"], &b"test"[..]),
        (vec!["rtlcompress", "--max-input", "3"], &b"test"[..]),
        (vec!["rtlcompress", "--max-output", "1"], &b"test"[..]),
        (vec!["rtldecompress", "--output-size", "10"], &[1][..]),
        (
            vec!["rtldecompress", "--output-size", "10", "--max-output", "9"],
            &[][..],
        ),
        (vec!["rtldecompress"], &[][..]),
        (
            vec!["rtlcompress", "--format", "xpress-huffman"],
            &b"short"[..],
        ),
    ] {
        let result = run(&args, input);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(!result.stderr.is_empty());
    }
    let encoded = run(&["rtlcompress"], b"abc");
    let result = run(&["rtldecompress", "--output-size", "4"], &encoded.stdout);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}
