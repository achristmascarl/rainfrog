use std::{env, path::Path, process::Command};

use color_eyre::eyre::{Result, WrapErr, bail, eyre};

#[derive(Debug, PartialEq, Eq)]
struct EditorCommand {
  program: String,
  args: Vec<String>,
}

fn editor_command_with(lookup: &impl Fn(&str) -> Option<String>) -> Result<EditorCommand> {
  let editor = lookup("VISUAL")
    .filter(|value| !value.trim().is_empty())
    .or_else(|| lookup("EDITOR").filter(|value| !value.trim().is_empty()))
    .unwrap_or_else(|| "vi".to_string());
  let mut parts = editor.split_whitespace();
  let program = parts.next().ok_or_else(|| eyre!("Could not parse editor command"))?.to_string();
  let args = parts.map(str::to_string).collect();
  Ok(EditorCommand { program, args })
}

pub fn open_editor(path: &Path) -> Result<()> {
  let command = editor_command_with(&|name| env::var(name).ok())?;
  run_editor(&command, path)
}

fn run_editor(command: &EditorCommand, path: &Path) -> Result<()> {
  let status = Command::new(&command.program)
    .args(&command.args)
    .arg(path)
    .status()
    .wrap_err_with(|| format!("Failed to launch external editor '{}'", command.program))?;
  if !status.success() {
    bail!("External editor '{}' exited with {status}", command.program);
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn env_lookup<'a>(values: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| values.iter().find_map(|(key, value)| (*key == name).then(|| value.to_string()))
  }

  #[test]
  fn visual_takes_precedence_over_editor() {
    let command =
      editor_command_with(&env_lookup(&[("VISUAL", "nvim"), ("EDITOR", "vim")])).unwrap();

    assert_eq!(command.program, "nvim");
    assert!(command.args.is_empty());
  }

  #[test]
  fn empty_editor_variables_are_ignored() {
    let command =
      editor_command_with(&env_lookup(&[("VISUAL", "  "), ("EDITOR", "vim -f")])).unwrap();

    assert_eq!(command.program, "vim");
    assert_eq!(command.args, ["-f"]);
  }

  #[test]
  fn vi_is_used_when_editor_variables_are_missing() {
    let command = editor_command_with(&env_lookup(&[])).unwrap();

    assert_eq!(command.program, "vi");
    assert!(command.args.is_empty());
  }

  #[test]
  fn editor_command_supports_simple_arguments() {
    let command = editor_command_with(&env_lookup(&[("VISUAL", "code --wait")])).unwrap();

    assert_eq!(command.program, "code");
    assert_eq!(command.args, ["--wait"]);
  }

  #[test]
  fn launch_error_preserves_os_error() {
    let missing_editor = env::temp_dir().join(format!(
      "rainfrog-missing-editor-{}-{}",
      std::process::id(),
      std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let command =
      EditorCommand { program: missing_editor.to_string_lossy().into_owned(), args: Vec::new() };

    let error = run_editor(&command, Path::new("query.sql")).unwrap_err();

    assert_eq!(
      error.downcast_ref::<std::io::Error>().map(std::io::Error::kind),
      Some(std::io::ErrorKind::NotFound)
    );
    assert!(format!("{error:#}").contains(&command.program));
  }
}
