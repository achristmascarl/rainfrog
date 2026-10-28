#![cfg(unix)]

use std::{fs, process::Command};

fn rainfrog_command() -> Command {
  // Cargo runs the test through its target runner, but subprocesses need to
  // select that runner explicitly too (for example, Cross's Android QEMU runner).
  let target = env!("VERGEN_CARGO_TARGET_TRIPLE").replace('-', "_").to_uppercase();
  let runner = std::env::var(format!("CARGO_TARGET_{target}_RUNNER")).unwrap_or_default();
  let mut runner_args = runner.split_whitespace();
  if let Some(program) = runner_args.next() {
    let mut command = Command::new(program);
    command.args(runner_args).arg(env!("CARGO_BIN_EXE_rainfrog"));
    command
  } else {
    Command::new(env!("CARGO_BIN_EXE_rainfrog"))
  }
}

#[test]
fn edit_command_creates_a_config_and_preserves_existing_edits_on_reopen() {
  let dir = tempfile::tempdir().unwrap();
  let config_dir = dir.path().join("config");
  let script = dir.path().join("editor.sh");
  fs::write(&script, "printf '\\n# edited by external editor\\n' >> \"$1\"\n").unwrap();

  for expected_edits in 1..=2 {
    let output = rainfrog_command()
      .arg("edit")
      .current_dir(dir.path())
      .env("RAINFROG_CONFIG", &config_dir)
      .env("VISUAL", format!("sh {}", script.display()))
      .env("EDITOR", "missing-editor")
      .output()
      .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let contents = fs::read_to_string(config_dir.join("rainfrog_config.toml")).unwrap();
    let config: toml::Value = toml::from_str(&contents).unwrap();
    assert!(config.get("keybindings").is_some());
    assert_eq!(contents.matches("# edited by external editor").count(), expected_edits);
  }
}

#[test]
fn edit_command_reports_editor_failure_and_leaves_the_config_available() {
  let dir = tempfile::tempdir().unwrap();
  let script = dir.path().join("editor.sh");
  fs::write(&script, "exit 7\n").unwrap();
  let output = rainfrog_command()
    .arg("edit")
    .current_dir(dir.path())
    .env("RAINFROG_CONFIG", dir.path().join("config"))
    .env("VISUAL", " ")
    .env("EDITOR", format!("sh {}", script.display()))
    .output()
    .unwrap();
  assert!(!output.status.success());
  let error = String::from_utf8_lossy(&output.stderr);
  assert!(error.contains("External editor 'sh' exited with"), "{error}");
  assert!(dir.path().join("config/rainfrog_config.toml").exists());
}
