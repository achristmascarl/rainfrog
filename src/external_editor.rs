use std::{env, fs, io::Write, path::Path, process::Command};

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

pub fn edit_query(query: &str) -> Result<String> {
  edit_query_with(query, open_editor)
}

fn edit_query_with<F>(query: &str, launch: F) -> Result<String>
where
  F: FnOnce(&Path) -> Result<()>,
{
  let mut file = tempfile::Builder::new()
    .prefix("rainfrog-query-")
    .suffix(".sql")
    .tempfile()
    .wrap_err("Failed to create temporary query file")?;
  file.write_all(query.as_bytes()).wrap_err("Failed to write query to temporary file")?;
  file.flush().wrap_err("Failed to flush query to temporary file")?;
  launch(file.path())?;
  fs::read_to_string(file.path()).wrap_err("Failed to read query from temporary file")
}

pub fn query_lines(text: &str) -> Vec<String> {
  let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
  if lines.is_empty() { vec![String::new()] } else { lines }
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
  fn empty_query_stays_as_one_editor_line() {
    assert_eq!(query_lines(""), [""]);
  }

  #[test]
  fn multiline_query_is_split_into_editor_lines() {
    assert_eq!(query_lines("select 1;\n\nselect 2;\n"), ["select 1;", "", "select 2;"]);
  }

  #[test]
  fn edit_query_round_trips_sql_through_a_temporary_file() {
    let edited = edit_query_with("select 1;", |path| {
      assert_eq!(path.extension().and_then(|value| value.to_str()), Some("sql"));
      assert_eq!(fs::read_to_string(path)?, "select 1;");
      fs::write(path, "select * from robot;")?;
      Ok(())
    })
    .unwrap();

    assert_eq!(edited, "select * from robot;");
  }

  #[test]
  fn read_error_identifies_the_temporary_file_operation() {
    let error = edit_query_with("select 1;", |path| {
      fs::write(path, [0xff])?;
      Ok(())
    })
    .unwrap_err();

    assert_eq!(
      error.downcast_ref::<std::io::Error>().map(std::io::Error::kind),
      Some(std::io::ErrorKind::InvalidData)
    );
    assert!(format!("{error:#}").starts_with("Failed to read query from temporary file: "));
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
