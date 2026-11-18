//! The standalone CLI (#645): every subcommand over the gate
//! fixtures through the compiled binary (CARGO_BIN_EXE), including
//! the error path and stdin input.

use std::process::{Command, Stdio};

fn run(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_expressir"))
        .args(args)
        .output()
        .expect("binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn cli_version_and_help() {
    let (out, _, ok) = run(&["version"]);
    assert!(ok);
    assert!(out.starts_with("expressir "), "version line: {out}");
    let (out, _, ok) = run(&["help"]);
    assert!(ok && out.contains("USAGE:"));
}

#[test]
fn cli_parse_and_format() {
    let path = fixture("eval_gate.exp");
    let (out, _, ok) = run(&["parse", &path]);
    assert!(ok);
    assert!(out.contains("\"_class\""));
    let (formatted, _, ok) = run(&["format", &path]);
    assert!(ok && formatted.contains("SCHEMA eval_gate;"));
    let (stripped, _, ok) = run(&["format-stripped", &path]);
    assert!(ok && !stripped.is_empty());
}

#[test]
fn cli_check_and_resolve() {
    let path = fixture("eval_gate.exp");
    let (out, _, ok) = run(&["check", &path]);
    assert!(ok && out.contains("\"id\""));
    let (out, _, ok) = run(&["resolve", &path]);
    assert!(ok && out.contains("_gate_fills"));
}

#[test]
fn cli_lsp_engines() {
    let path = fixture("eval_gate.exp");
    let (out, _, ok) = run(&["lsp-symbols", &path]);
    assert!(ok && out.contains("\"name\""));
    let (out, _, ok) = run(&["lsp-inventory", &path]);
    assert!(ok && out.contains("declarations"));
}

#[test]
fn cli_p21_round_trip() {
    let path = fixture("eval_rules_gate.p21");
    let (out, _, ok) = run(&["p21-parse", &path]);
    assert!(ok && out.contains("instances"));
    let (out, _, ok) = run(&["p21-write", &path]);
    assert!(ok && out.contains("ISO-10303-21;"));
}

#[test]
fn cli_exports() {
    let path = fixture("eval_gate.exp");
    let (out, _, ok) = run(&["owl", &path]);
    assert!(ok && out.contains("@prefix"));
    let (out, _, ok) = run(&["json-schema", &path]);
    assert!(ok && out.contains("$defs"));
}

#[test]
fn cli_stdin_and_errors() {
    let source = std::fs::read_to_string(fixture("eval_gate.exp")).unwrap();
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_expressir"))
        .args(["format", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(source.as_bytes())
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("SCHEMA eval_gate;"));

    let (_, err, ok) = run(&["nope"]);
    assert!(!ok && err.contains("unknown command"));
    let (_, err, ok) = run(&["format"]);
    assert!(!ok && err.contains("missing input"));
}
