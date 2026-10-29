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

pub async fn edit_query(query: String) -> Result<String> {
  edit_query_with(query, open_editor).await
}

async fn edit_query_with<F>(query: String, launch: F) -> Result<String>
where
  F: FnOnce(&Path) -> Result<()> + Send + 'static,
{
  tokio::task::spawn_blocking(move || {
    let mut file = tempfile::Builder::new()
      .prefix("rainfrog-query-")
      .suffix(".sql")
      .tempfile()
      .wrap_err("Failed to create temporary query file")?;
    file.write_all(query.as_bytes()).wrap_err("Failed to write query to temporary file")?;
    file.flush().wrap_err("Failed to flush query to temporary file")?;
    launch(file.path())?;
    fs::read_to_string(file.path()).wrap_err("Failed to read query from temporary file")
  })
  .await
  .wrap_err("External editor task failed")?
}

pub fn query_lines(text: &str) -> Vec<String> {
  let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
  if lines.is_empty() { vec![String::new()] } else { lines }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[cfg(unix)]
  fn env_lookup<'a>(values: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| values.iter().find_map(|(key, value)| (*key == name).then(|| value.to_string()))
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn selected_editor_process_edits_the_query_and_cleans_up_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("editor.sh");
    fs::write(&script, "test \"$(cat \"$1\")\" = 'select 1;' || exit 1\nprintf 'select 2;\\n\\nselect 3;\\n' > \"$1\"\n").unwrap();
    let visual = format!("sh {}", script.display());
    let command =
      editor_command_with(&env_lookup(&[("VISUAL", &visual), ("EDITOR", "missing-editor")]))
        .unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let edited = edit_query_with("select 1;".into(), move |path| {
      tx.send(path.to_path_buf()).unwrap();
      run_editor(&command, path)
    })
    .await
    .unwrap();
    assert_eq!(query_lines(&edited), ["select 2;", "", "select 3;"]);
    assert!(!rx.await.unwrap().exists());
  }

  #[tokio::test]
  async fn edit_query_round_trips_sql_through_a_temporary_file() {
    let edited = edit_query_with("select 1;".to_string(), |path| {
      assert_eq!(path.extension().and_then(|value| value.to_str()), Some("sql"));
      assert_eq!(fs::read_to_string(path)?, "select 1;");
      fs::write(path, "select * from robot;")?;
      Ok(())
    })
    .await
    .unwrap();

    assert_eq!(edited, "select * from robot;");
  }

  #[tokio::test(flavor = "current_thread")]
  async fn editing_query_keeps_async_runtime_responsive() {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    let edit = edit_query_with("select 1;".to_string(), move |path| {
      started_tx.send(()).unwrap();
      release_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .wrap_err("Async runtime did not progress while the editor was open")?;
      fs::write(path, "select * from robot;")?;
      Ok(())
    });
    let release_editor = async move {
      started_rx.await.unwrap();
      release_tx.send(()).unwrap();
    };

    let (edited, ()) = tokio::join!(edit, release_editor);

    assert_eq!(edited.unwrap(), "select * from robot;");
  }

  #[tokio::test]
  async fn read_error_identifies_the_temporary_file_operation() {
    let error = edit_query_with("select 1;".to_string(), |path| {
      fs::write(path, [0xff])?;
      Ok(())
    })
    .await
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
