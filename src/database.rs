use std::{collections::HashMap, fmt::Write, pin::Pin, string::String};

use futures::stream::{BoxStream, StreamExt};
use sqlparser::{
  ast::Statement,
  dialect::PostgreSqlDialect,
  keywords,
  parser::{Parser, ParserError},
};
use sqlx::{
  postgres::{PgColumn, PgPool, PgPoolOptions, PgQueryResult, PgRow, PgTypeInfo, PgTypeKind, PgValueRef, Postgres},
  types::Uuid,
  Column, Database, Either, Error, Pool, Row, Transaction, ValueRef,
};

#[derive(Debug)]
pub struct Header {
  pub name: String,
  pub type_name: String,
}

pub struct Value {
  pub is_null: bool,
  pub string: String,
}

#[derive(Debug)]
pub struct Rows {
  pub headers: Headers,
  pub rows: Vec<Vec<String>>,
  pub rows_affected: Option<u64>,
}
pub type Headers = Vec<Header>;
pub type DbPool = PgPool;
pub type DbError = sqlx::Either<Error, ParserError>;

pub async fn init_pool(url: String) -> Result<PgPool, Error> {
  PgPoolOptions::new().max_connections(5).connect(&url).await
}

// since it's possible for raw_sql to execute multiple queries in a single string,
// we only execute the first one and then drop the rest.
pub async fn query(query: String, pool: &PgPool) -> Result<Rows, DbError> {
  let first_query = get_first_query(query);
  match first_query {
    Ok((first_query, _)) => {
      let stream = sqlx::raw_sql(&first_query).fetch_many(pool);
      query_stream(stream).await
    },
    Err(e) => Err(e),
  }
}

pub async fn query_stream(
  mut stream: BoxStream<'_, Result<Either<PgQueryResult, PgRow>, Error>>,
) -> Result<Rows, DbError> {
  let mut query_finished = false;
  let mut query_rows = vec![];
  let mut query_rows_affected: Option<u64> = None;
  let mut headers: Headers = vec![];
  while !query_finished {
    let next = stream.next().await;
    match next {
      Some(Ok(Either::Left(result))) => {
        query_rows_affected = Some(result.rows_affected());
        query_finished = true;
      },
      Some(Ok(Either::Right(row))) => {
        query_rows.push(row_to_vec(&row));
        if headers.is_empty() {
          headers = get_headers(&row);
        }
      },
      Some(Err(e)) => return Err(Either::Left(e)),
      None => return Err(Either::Left(Error::Protocol("Results stream empty".to_owned()))),
    };
  }
  Ok(Rows { rows_affected: query_rows_affected, headers, rows: query_rows })
}

pub async fn query_with_tx<'a>(
  mut tx: Transaction<'_, Postgres>,
  query: String,
) -> (Result<Either<u64, Rows>, DbError>, Transaction<'_, Postgres>) {
  let first_query = get_first_query(query);
  match first_query {
    Ok((first_query, statement_type)) => {
      match statement_type {
        Statement::Explain { .. } => {
          let stream = sqlx::raw_sql(&first_query).fetch_many(&mut *tx);
          let result = query_stream(stream).await;
          match result {
            Ok(result) => (Ok(Either::Right(result)), tx),
            Err(e) => (Err(e), tx),
          }
        },
        _ => {
          let result = sqlx::query(&first_query).execute(&mut *tx).await;
          match result {
            Ok(result) => (Ok(Either::Left(result.rows_affected())), tx),
            Err(e) => (Err(DbError::Left(e)), tx),
          }
        },
      }
    },
    Err(e) => (Err(e), tx),
  }
}

pub fn get_first_query(query: String) -> Result<(String, Statement), DbError> {
  let dialect = PostgreSqlDialect {};
  let ast = Parser::parse_sql(&dialect, &query);
  match ast {
    Ok(ast) if ast.len() > 1 => {
      Err(Either::Right(ParserError::ParserError("Only one statement allowed per query".to_owned())))
    },
    Ok(ast) if ast.is_empty() => Err(Either::Right(ParserError::ParserError("Parsed query is empty".to_owned()))),
    Ok(ast) => {
      let statement = ast[0].clone();
      Ok((statement.to_string(), statement))
    },
    Err(e) => Err(Either::Right(e)),
  }
}

pub fn statement_type_string(statement: &Statement) -> String {
  format!("{:?}", statement).split('(').collect::<Vec<&str>>()[0].split('{').collect::<Vec<&str>>()[0]
    .split('[')
    .collect::<Vec<&str>>()[0]
    .trim()
    .to_string()
}

pub fn should_use_tx(statement: Statement) -> bool {
  match statement {
    Statement::Delete(_) | Statement::Drop { .. } | Statement::Update { .. } => true,
    Statement::Explain { statement, analyze, .. }
      if analyze
        && matches!(statement.as_ref(), Statement::Delete(_) | Statement::Drop { .. } | Statement::Update { .. }) =>
    {
      true
    },
    Statement::Explain { .. } => false,
    _ => false,
  }
}

pub fn get_headers(row: &PgRow) -> Headers {
  row
    .columns()
    .iter()
    .map(|col| Header { name: col.name().to_string(), type_name: col.type_info().to_string() })
    .collect()
}

// parsed based on https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html
pub fn parse_value(row: &PgRow, col: &PgColumn) -> Option<Value> {
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

pub fn row_to_json(row: &PgRow) -> HashMap<String, String> {
  let mut result = HashMap::new();
  for col in row.columns() {
    let value = match parse_value(row, col) {
      Some(v) => v.string,
      _ => "[ unsupported ]".to_string(),
    };
    result.insert(col.name().to_string(), value);
  }

  result
}

pub fn vec_to_string<T: std::string::ToString>(vec: Vec<T>) -> String {
  vec.iter().fold(String::new(), |mut output, b| {
    let s = b.to_string();
    let _ = write!(output, "{s}");
    output
  })
}

pub fn row_to_vec(row: &PgRow) -> Vec<String> {
  row.columns().iter().map(|col| parse_value(row, col).unwrap().string).collect()
}

pub fn get_keywords() -> Vec<String> {
  keywords::ALL_KEYWORDS.iter().map(|k| k.to_string()).collect()
}

#[cfg(test)]
mod tests {
  use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};

  use super::*;

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
      let result = get_first_query(input.to_string());
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
      assert_eq!(should_use_tx(statement), expected, "Failed for query: {}", query);
    }
  }
}
