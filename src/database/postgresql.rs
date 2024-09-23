use std::{collections::HashMap, fmt::Write, pin::Pin, string::String};

use futures::stream::{BoxStream, StreamExt};
use sqlparser::{
  ast::Statement,
  dialect::PostgreSqlDialect,
  keywords,
  parser::{Parser, ParserError},
};
use sqlx::{
  postgres::{
    PgColumn, PgConnectOptions, PgPool, PgPoolOptions, PgQueryResult, PgRow, PgTypeInfo, PgTypeKind, PgValueRef,
    Postgres,
  },
  types::Uuid,
  Column, Database, Either, Error, Pool, Row, Transaction, ValueRef,
};

use super::{vec_to_string, Value};

impl super::ValueParser for Postgres {
  fn parse_value(row: &<Postgres as sqlx::Database>::Row, col: &<Postgres as sqlx::Database>::Column) -> Option<Value> {
    let col_type = col.type_info().to_string();
    let raw_value = row.try_get_raw(col.ordinal()).unwrap();
    if raw_value.is_null() {
      return Some(Value { string: "NULL".to_string(), is_null: true });
    }
    match col_type.to_uppercase().as_str() {
      "TIMESTAMPTZ" => {
        let received: chrono::DateTime<chrono::Utc> = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "TIMESTAMP" => {
        let received: chrono::NaiveDateTime = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "DATE" => {
        let received: chrono::NaiveDate = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "TIME" => {
        let received: chrono::NaiveTime = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "UUID" => {
        let received: Uuid = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "INET" | "CIDR" => {
        let received: std::net::IpAddr = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "JSON" | "JSONB" => {
        let received: serde_json::Value = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "BOOL" => {
        let received: bool = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "SMALLINT" | "SMALLSERIAL" | "INT2" => {
        let received: i16 = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "INT" | "SERIAL" | "INT4" => {
        let received: i32 = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "BIGINT" | "BIGSERIAL" | "INT8" => {
        let received: i64 = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "REAL" | "FLOAT4" => {
        let received: f32 = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "DOUBLE PRECISION" | "FLOAT8" => {
        let received: f64 = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received.to_string(), is_null: false })
      },
      "TEXT" | "VARCHAR" | "NAME" | "CITEXT" | "BPCHAR" | "CHAR" => {
        let received: String = row.try_get(col.ordinal()).unwrap();
        Some(Value { string: received, is_null: false })
      },
      "BYTEA" => {
        let received: Vec<u8> = row.try_get(col.ordinal()).unwrap();
        Some(Value {
          string: received.iter().fold(String::new(), |mut output, b| {
            let _ = write!(output, "{b:02X}");
            output
          }),
          is_null: false,
        })
      },
      "VOID" => Some(Value { string: "".to_string(), is_null: false }),
      _ if col_type.to_uppercase().ends_with("[]") => {
        let array_type = col_type.to_uppercase().replace("[]", "");
        match array_type.as_str() {
          "TIMESTAMPTZ" => {
            let received: Vec<chrono::DateTime<chrono::Utc>> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "TIMESTAMP" => {
            let received: Vec<chrono::NaiveDateTime> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "DATE" => {
            let received: Vec<chrono::NaiveDate> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "TIME" => {
            let received: Vec<chrono::NaiveTime> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "UUID" => {
            let received: Vec<Uuid> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "INET" | "CIDR" => {
            let received: Vec<std::net::IpAddr> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "JSON" | "JSONB" => {
            let received: Vec<serde_json::Value> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "BOOL" => {
            let received: Vec<bool> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "SMALLINT" | "SMALLSERIAL" | "INT2" => {
            let received: Vec<i16> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "INT" | "SERIAL" | "INT4" => {
            let received: Vec<i32> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "BIGINT" | "BIGSERIAL" | "INT8" => {
            let received: Vec<i64> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "REAL" | "FLOAT4" => {
            let received: Vec<f32> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "DOUBLE PRECISION" | "FLOAT8" => {
            let received: Vec<f64> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "TEXT" | "VARCHAR" | "NAME" | "CITEXT" | "BPCHAR" | "CHAR" => {
            let received: Vec<String> = row.try_get(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
          "BYTEA" => {
            let received: Vec<u8> = row.try_get(col.ordinal()).unwrap();
            Some(Value {
              string: received.iter().fold(String::new(), |mut output, b| {
                let _ = write!(output, "{b:02X}");
                output
              }),
              is_null: false,
            })
          },
          _ => {
            // try to cast custom or other types to strings
            let received: Vec<String> = row.try_get_unchecked(col.ordinal()).unwrap();
            Some(Value { string: vec_to_string(received), is_null: false })
          },
        }
      },
      _ => {
        // try to cast custom or other types to strings
        let received: String = row.try_get_unchecked(col.ordinal()).unwrap();
        Some(Value { string: received, is_null: false })
      },
    }
  }
}
mod tests {
  use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};

  use super::*;
  use crate::database::{get_first_query, DbError};

  #[test]
  fn test_get_first_query() {
    type TestCase = (&'static str, Result<(String, Box<dyn Fn(Statement) -> bool>), DbError>);

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
        "select * from \"public\".\"users\"",
        Ok(("SELECT * FROM \"public\".\"users\"".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
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
      (
        "-- select blah;\nselect * from users\n-- insert blah",
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
        Ok(("DELETE FROM users WHERE id = 1".to_owned(), Box::new(|s| matches!(s, Statement::Delete(_))))),
      ),
      // drop
      ("DROP TABLE users", Ok(("DROP TABLE users".to_owned(), Box::new(|s| matches!(s, Statement::Drop { .. }))))),
      // explain
      (
        "EXPLAIN SELECT * FROM users",
        Ok(("EXPLAIN SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Explain { .. })))),
      ),
    ];

    for (input, expected_output) in test_cases {
      let result = get_first_query(input.to_string(), &PostgreSqlDialect {});
      match (result, expected_output) {
        (Ok((query, statement_type)), Ok((expected_query, match_statement))) => {
          assert_eq!(query, expected_query);
          assert!(match_statement(statement_type));
        },
        (
          Err(Either::Right(ParserError::ParserError(msg))),
          Err(Either::Right(ParserError::ParserError(expected_msg))),
        ) => {
          assert_eq!(msg, expected_msg)
        },
        _ => panic!("Unexpected result for input: {}", input),
      }
    }
  }

  #[test]
  fn test_should_use_tx() {
    let dialect = PostgreSqlDialect {};
    let test_cases = vec![
      ("DELETE FROM users WHERE id = 1", true),
      ("DROP TABLE users", true),
      ("UPDATE users SET name = 'John' WHERE id = 1", true),
      ("SELECT * FROM users", false),
      ("INSERT INTO users (name) VALUES ('John')", false),
      ("EXPLAIN ANALYZE DELETE FROM users WHERE id = 1", true),
      ("EXPLAIN SELECT * FROM users", false),
      ("EXPLAIN ANALYZE SELECT * FROM users WHERE id = 1", false),
    ];

    for (query, expected) in test_cases {
      let ast = Parser::parse_sql(&dialect, query).unwrap();
      let statement = ast[0].clone();
      // assert_eq!(should_use_tx(statement), expected, "Failed for query: {}", query);
    }
  }
}
