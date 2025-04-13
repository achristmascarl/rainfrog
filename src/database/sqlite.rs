use std::{
  fmt::Write,
  io::{self, Write as _},
  str::FromStr,
  string::String,
};

use serde_json;
use sqlx::{
  sqlite::{Sqlite, SqliteConnectOptions, SqliteQueryResult},
  types::{
    chrono,
    uuid::{self, Timestamp},
    Uuid,
  },
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
            return Err(eyre::Report::msg("Database file path is required"));
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

impl super::ValueParser for Sqlite {
  fn parse_value(row: &<Sqlite as sqlx::Database>::Row, col: &<Sqlite as sqlx::Database>::Column) -> Option<Value> {
    let col_type = col.type_info().to_string();
    if row.try_get_raw(col.ordinal()).is_ok_and(|v| v.is_null()) {
      return Some(Value { parse_error: false, string: "NULL".to_string(), is_null: true });
    }
    match col_type.to_uppercase().as_str() {
      "BOOLEAN" => {
        Some(
          row
            .try_get::<bool, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "INTEGER" | "INT4" | "INT8" | "BIGINT" => {
        Some(
          row
            .try_get::<i64, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "REAL" => {
        Some(
          row
            .try_get::<f64, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TEXT" => {
        // Try parsing as different types that might be stored as TEXT
        if let Ok(dt) = row.try_get::<chrono::NaiveDateTime, _>(col.ordinal()) {
          Some(Value { parse_error: false, string: dt.to_string(), is_null: false })
        } else if let Ok(dt) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(col.ordinal()) {
          Some(Value { parse_error: false, string: dt.to_string(), is_null: false })
        } else if let Ok(date) = row.try_get::<chrono::NaiveDate, _>(col.ordinal()) {
          Some(Value { parse_error: false, string: date.to_string(), is_null: false })
        } else if let Ok(time) = row.try_get::<chrono::NaiveTime, _>(col.ordinal()) {
          Some(Value { parse_error: false, string: time.to_string(), is_null: false })
        } else if let Ok(uuid) = row.try_get::<uuid::Uuid, _>(col.ordinal()) {
          Some(Value { parse_error: false, string: uuid.to_string(), is_null: false })
        } else if let Ok(json) = row.try_get::<serde_json::Value, _>(col.ordinal()) {
          Some(Value { parse_error: false, string: json.to_string(), is_null: false })
        } else if let Ok(string) = row.try_get::<String, _>(col.ordinal()) {
          Some(Value { parse_error: false, string, is_null: false })
        } else {
          Some(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false })
        }
      },
      "BLOB" => {
        Some(row.try_get::<Vec<u8>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| {
            if let Ok(s) = String::from_utf8(received.clone()) {
              Value { parse_error: false, string: s, is_null: false }
            } else {
              Value {
                parse_error: false,
                string: received.iter().fold(String::new(), |mut output, b| {
                  let _ = write!(output, "{b:02X}");
                  output
                }),
                is_null: false,
              }
            }
          },
        ))
      },
      "DATETIME" => {
        // Similar to TEXT, but we'll try timestamp first
        if let Ok(dt) = row.try_get::<i64, _>(col.ordinal()) {
          Some(
            chrono::DateTime::from_timestamp(dt, 0)
              .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                Value { parse_error: false, string: received.to_string(), is_null: false }
              }),
          )
        } else if let Ok(dt) = row.try_get::<chrono::NaiveDateTime, _>(col.ordinal()) {
          Some(Value { parse_error: true, string: dt.to_string(), is_null: false })
        } else if let Ok(dt) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(col.ordinal()) {
          Some(Value { parse_error: true, string: dt.to_string(), is_null: false })
        } else {
          Some(
            row
              .try_get::<String, usize>(col.ordinal())
              .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                Value { parse_error: false, string: received.to_string(), is_null: false }
              }),
          )
        }
      },
      "DATE" => {
        if let Ok(date) = row.try_get::<chrono::NaiveDate, _>(col.ordinal()) {
          Some(Value { parse_error: true, string: date.to_string(), is_null: false })
        } else {
          Some(
            row
              .try_get::<String, usize>(col.ordinal())
              .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                Value { parse_error: false, string: received.to_string(), is_null: false }
              }),
          )
        }
      },
      "TIME" => {
        if let Ok(time) = row.try_get::<chrono::NaiveTime, _>(col.ordinal()) {
          Some(Value { parse_error: true, string: time.to_string(), is_null: false })
        } else {
          Some(
            row
              .try_get::<String, usize>(col.ordinal())
              .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                Value { parse_error: false, string: received.to_string(), is_null: false }
              }),
          )
        }
      },
      _ => {
        // For any other types, try to cast to string
        Some(
          row
            .try_get_unchecked::<String, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
    }
  }
}

mod tests {
  use sqlparser::{
    ast::Statement,
    dialect::SQLiteDialect,
    parser::{Parser, ParserError},
  };

  use super::*;
  use crate::database::{get_execution_type, get_first_query, DbError, ExecutionType};

  #[test]
  fn test_get_first_query_sqlite() {
    type TestCase = (&'static str, Result<(String, Box<dyn Fn(&Statement) -> bool>), DbError>);

    let test_cases: Vec<TestCase> = vec![
      // single query
      ("SELECT * FROM users;", Ok(("SELECT * FROM users".to_string(), Box::new(|s| matches!(s, Statement::Query(_)))))),
      // multiple queries
      (
        "SELECT * FROM users; DELETE FROM posts;",
        Err(DbError::Right(ParserError::ParserError("Only one statement allowed per query".to_owned()))),
      ),
      // empty query
      ("", Err(DbError::Right(ParserError::ParserError("Parsed query is empty".to_owned())))),
      // syntax error
      (
        "SELEC * FORM users;",
        Err(DbError::Right(ParserError::ParserError(
          "Expected: an SQL statement, found: SELEC at Line: 1, Column: 1".to_owned(),
        ))),
      ),
      // lowercase
      (
        "select * from \"users\"",
        Ok(("SELECT * FROM \"users\"".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
      ),
      // newlines
      ("select *\nfrom users;", Ok(("SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Query(_)))))),
      // comment-only
      ("-- select * from users;", Err(DbError::Right(ParserError::ParserError("Parsed query is empty".to_owned())))),
      // commented line(s)
      (
        "-- select blah;\nselect * from users",
        Ok(("SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
      ),
      // update
      (
        "UPDATE users SET name = 'John' WHERE id = 1",
        Ok((
          "UPDATE users SET name = 'John' WHERE id = 1".to_owned(),
          Box::new(|s| matches!(s, Statement::Update { .. })),
        )),
      ),
      // delete
      (
        "DELETE FROM users WHERE id = 1",
        Ok(("DELETE FROM users WHERE id = 1".to_owned(), Box::new(|s| matches!(s, Statement::Delete { .. })))),
      ),
      // drop
      ("DROP TABLE users", Ok(("DROP TABLE users".to_owned(), Box::new(|s| matches!(s, Statement::Drop { .. }))))),
      // explain
      (
        "EXPLAIN SELECT * FROM users",
        Ok(("EXPLAIN SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Explain { .. })))),
      ),
    ];

    let dialect = Box::new(SQLiteDialect {});

    for (input, expected_output) in test_cases {
      let result = get_first_query(input.to_string(), dialect.as_ref());
      match (result, expected_output) {
        (Ok((query, statement)), Ok((expected_query, match_statement))) => {
          assert_eq!(query, expected_query);
          assert!(match_statement(&statement));
        },
        (
          Err(DbError::Right(ParserError::ParserError(msg))),
          Err(DbError::Right(ParserError::ParserError(expected_msg))),
        ) => {
          assert_eq!(msg, expected_msg);
        },
        _ => panic!("Unexpected result for input: {}", input),
      }
    }
  }

  #[test]
  fn test_execution_type_sqlite() {
    let dialect = SQLiteDialect {};
    let test_cases = vec![
      ("DELETE FROM users WHERE id = 1", ExecutionType::Transaction),
      ("DROP TABLE users", ExecutionType::Confirm),
      ("UPDATE users SET name = 'John' WHERE id = 1", ExecutionType::Transaction),
      ("SELECT * FROM users", ExecutionType::Normal),
      ("INSERT INTO users (name) VALUES ('John')", ExecutionType::Normal),
      ("EXPLAIN DELETE FROM users WHERE id = 1", ExecutionType::Normal),
      ("EXPLAIN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN QUERY PLAN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN Query PLAN DELETE FROM users WHERE id = 1", ExecutionType::Normal),
    ];

    for (query, expected) in test_cases {
      let ast = Parser::parse_sql(&dialect, query).unwrap();
      let statement = ast[0].clone();
      assert_eq!(get_execution_type(statement, false), expected, "Failed for query: {}", query);
    }
  }
}
