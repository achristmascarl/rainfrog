use std::{path::Path, time::Duration};

use chrono::{DateTime, Local, Utc};
use color_eyre::eyre::Result;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions, types::Json};

pub const HISTORY_LIMIT: usize = 50;

#[derive(Debug, PartialEq)]
pub struct HistoryEntry {
  pub query_lines: Vec<String>,
  pub timestamp: DateTime<Local>,
}

pub struct HistoryStore {
  connection: SqliteConnection,
}

impl HistoryStore {
  pub async fn open(data_dir: &Path) -> Result<Self> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("history.sqlite3");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      options.mode(0o600);
    }
    options.open(&path)?;

    let mut connection = SqliteConnection::connect_with(
      &SqliteConnectOptions::new().filename(path).busy_timeout(Duration::from_millis(250)),
    )
    .await?;
    sqlx::query(
      "CREATE TABLE IF NOT EXISTS history (
        id INTEGER PRIMARY KEY,
        query_lines TEXT NOT NULL,
        timestamp TEXT NOT NULL
      )",
    )
    .execute(&mut connection)
    .await?;
    Ok(Self { connection })
  }

  pub async fn load(&mut self) -> Result<Vec<HistoryEntry>> {
    let rows = sqlx::query_as::<_, (Json<Vec<String>>, DateTime<Utc>)>(
      "SELECT query_lines, timestamp FROM history ORDER BY id DESC LIMIT ?",
    )
    .bind(HISTORY_LIMIT as i64)
    .fetch_all(&mut self.connection)
    .await?;
    Ok(
      rows
        .into_iter()
        .map(|(query_lines, timestamp)| HistoryEntry {
          query_lines: query_lines.0,
          timestamp: timestamp.with_timezone(&Local),
        })
        .collect(),
    )
  }

  pub async fn append(&mut self, entry: &HistoryEntry) -> Result<()> {
    let mut transaction = self.connection.begin().await?;
    sqlx::query("INSERT INTO history (query_lines, timestamp) VALUES (?, ?)")
      .bind(Json(&entry.query_lines))
      .bind(entry.timestamp.with_timezone(&Utc))
      .execute(&mut *transaction)
      .await?;
    sqlx::query(
      "DELETE FROM history WHERE id NOT IN (SELECT id FROM history ORDER BY id DESC LIMIT ?)",
    )
    .bind(HISTORY_LIMIT as i64)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
  }

  pub async fn clear(&mut self) -> Result<()> {
    sqlx::query("DELETE FROM history").execute(&mut self.connection).await?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn entry(query: &str) -> HistoryEntry {
    HistoryEntry { query_lines: vec![query.to_string()], timestamp: chrono::Local::now() }
  }

  #[tokio::test]
  async fn retains_latest_50_entries_after_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path()).await.unwrap();
    for i in 0..55 {
      store.append(&entry(&format!("select {i}"))).await.unwrap();
    }
    drop(store);

    let mut store = HistoryStore::open(dir.path()).await.unwrap();
    let entries = store.load().await.unwrap();
    assert_eq!(entries.len(), 50);
    assert_eq!(entries[0].query_lines, ["select 54"]);
    assert_eq!(entries[49].query_lines, ["select 5"]);
    let stored_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history")
      .fetch_one(&mut store.connection)
      .await
      .unwrap();
    assert_eq!(stored_count, 50);
  }

  #[tokio::test]
  async fn concurrent_sessions_keep_both_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = HistoryStore::open(dir.path()).await.unwrap();
    let mut second = HistoryStore::open(dir.path()).await.unwrap();
    let first_entry = entry("select 1");
    let second_entry = entry("select 2");
    let (first_result, second_result) =
      tokio::join!(first.append(&first_entry), second.append(&second_entry));
    first_result.unwrap();
    second_result.unwrap();

    let mut queries: Vec<_> =
      first.load().await.unwrap().into_iter().map(|entry| entry.query_lines).collect();
    queries.sort();
    assert_eq!(queries, vec![vec!["select 1"], vec!["select 2"]]);
  }

  #[tokio::test]
  async fn clearing_history_does_not_resurrect_another_sessions_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = HistoryStore::open(dir.path()).await.unwrap();
    let mut second = HistoryStore::open(dir.path()).await.unwrap();
    first.append(&entry("select 1")).await.unwrap();
    assert_eq!(second.load().await.unwrap().len(), 1);
    first.clear().await.unwrap();
    second.append(&entry("select 2")).await.unwrap();

    let mut reopened = HistoryStore::open(dir.path()).await.unwrap();
    let entries = reopened.load().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].query_lines, ["select 2"]);
  }
}
