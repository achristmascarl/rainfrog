use std::{
  fmt::Write,
  io::{self, Write as _},
  str::FromStr,
  string::String,
};

use serde_json;
use sqlx::{
  sqlite::{Sqlite, SqliteConnectOptions, SqliteQueryResult},
  types::{chrono, uuid, Uuid},
  Column, Database, Row, ValueRef,
};

use super::{vec_to_string, Value};
use crate::cli::Cli;

impl super::BuildConnectionOptions for sqlx::Sqlite {
  fn build_connection_opts(args: Cli) -> color_eyre::eyre::Result<<Self::Connection as sqlx::Connection>::Options> {
    match args.connection_url {
      Some(url) => Ok(SqliteConnectOptions::from_str(&url)?),
      None => {
        let filename = if let Some(database) = args.database {
          database
        } else {
          let mut database = String::new();
          print!("database file path (or ':memory:'): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim().to_string();
          if database.is_empty() {
            return Err(color_eyre::eyre::Report::msg("Database file path is required"));
          }
          database
        };

        let opts = SqliteConnectOptions::new().filename(&filename);
        Ok(opts)
      },
    }
  }
}

impl super::DatabaseQueries for Sqlite {
  fn preview_tables_query() -> String {
    "select '' as table_schema, name as table_name
      from sqlite_master
      where type = 'table'
      and name not like 'sqlite_%'
      order by name asc"
      .to_owned()
  }

  fn preview_rows_query(_schema: &str, table: &str) -> String {
    format!("select * from \"{}\" limit 100", table)
  }

  fn preview_columns_query(_schema: &str, table: &str) -> String {
    format!("pragma table_info(\"{}\")", table)
  }

  fn preview_constraints_query(_schema: &str, table: &str) -> String {
    format!("pragma foreign_key_list(\"{}\")", table)
  }

  fn preview_indexes_query(_schema: &str, table: &str) -> String {
    format!("pragma index_list(\"{}\")", table)
  }

  fn preview_policies_query(_schema: &str, _table: &str) -> String {
    "select 'SQLite does not support row-level security policies' as message".to_owned()
  }
}

impl super::HasRowsAffected for SqliteQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}
// macro_rules! parse_nullable {
//   ($type:ty) => {
//     match row.try_get::<Option<$type>, _>(col.ordinal()) {
//       Ok(Some(value)) => Some(Value { string: value.to_string(), is_null: false }),
//       Ok(None) => Some(Value { string: "NULL".to_string(), is_null: true }),
//       Err(_) => None,
//     }
//   };
// }

impl super::ValueParser for Sqlite {
  fn parse_value(row: &<Sqlite as sqlx::Database>::Row, col: &<Sqlite as sqlx::Database>::Column) -> Option<Value> {
    let col_type = col.type_info().to_string();
    let raw_value = row.try_get_raw(col.ordinal()).unwrap();
    if raw_value.is_null() {
      return Some(Value { string: "NULL".to_string(), is_null: true });
    }
    match col_type.to_uppercase().as_str() {
      "BOOLEAN" => {
        let received: bool = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "INTEGER" | "INT4" | "INT8" | "BIGINT" => {
        let received: i64 = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "REAL" => {
        let received: f64 = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "TEXT" => {
        // Try parsing as different types that might be stored as TEXT
        if let Ok(dt) = row.try_get::<chrono::NaiveDateTime, _>(col.ordinal()) {
          Some(Value { string: dt.to_string(), is_null: false })
        } else if let Ok(dt) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(col.ordinal()) {
          Some(Value { string: dt.to_string(), is_null: false })
        } else if let Ok(date) = row.try_get::<chrono::NaiveDate, _>(col.ordinal()) {
          Some(Value { string: date.to_string(), is_null: false })
        } else if let Ok(time) = row.try_get::<chrono::NaiveTime, _>(col.ordinal()) {
          Some(Value { string: time.to_string(), is_null: false })
        } else if let Ok(uuid) = row.try_get::<uuid::Uuid, _>(col.ordinal()) {
          Some(Value { string: uuid.to_string(), is_null: false })
        } else if let Ok(json) = row.try_get::<serde_json::Value, _>(col.ordinal()) {
          Some(Value { string: json.to_string(), is_null: false })
        } else {
          let received: String = row.try_get(col.ordinal()).unwrap();
          Some(Value { string: received, is_null: false })
        }
      },
      "BLOB" => {
        let received: Vec<u8> = row.try_get(col.ordinal()).unwrap();
        if let Ok(s) = String::from_utf8(received.clone()) {
          Some(Value { string: s, is_null: false })
        } else {
          Some(Value {
            string: received.iter().fold(String::new(), |mut output, b| {
              let _ = write!(output, "{b:02X}");
              output
            }),
            is_null: false,
          })
        }
      },
      "DATETIME" => {
        // Similar to TEXT, but we'll try timestamp first
        if let Ok(dt) = row.try_get::<i64, _>(col.ordinal()) {
          let dt = chrono::DateTime::from_timestamp(dt, 0).unwrap();
          Some(Value { string: dt.to_string(), is_null: false })
        } else if let Ok(dt) = row.try_get::<chrono::NaiveDateTime, _>(col.ordinal()) {
          Some(Value { string: dt.to_string(), is_null: false })
        } else if let Ok(dt) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(col.ordinal()) {
          Some(Value { string: dt.to_string(), is_null: false })
        } else {
          let received: String = row.try_get(col.ordinal()).unwrap();
          Some(Value { string: received, is_null: false })
        }
      },
      "DATE" => {
        if let Ok(date) = row.try_get::<chrono::NaiveDate, _>(col.ordinal()) {
          Some(Value { string: date.to_string(), is_null: false })
        } else {
          let received: String = row.try_get(col.ordinal()).unwrap();
          Some(Value { string: received, is_null: false })
        }
      },
      "TIME" => {
        if let Ok(time) = row.try_get::<chrono::NaiveTime, _>(col.ordinal()) {
          Some(Value { string: time.to_string(), is_null: false })
        } else {
          let received: String = row.try_get(col.ordinal()).unwrap();
          Some(Value { string: received, is_null: false })
        }
      },
      _ => {
        // For any other types, try to cast to string
        let received: String = row.try_get_unchecked(col.ordinal()).unwrap();
        Some(Value { string: received, is_null: false })
      },
    }
  }
}
