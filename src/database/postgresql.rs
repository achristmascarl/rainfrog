use std::{
  fmt::Write,
  io::{self, Write as _},
  str::FromStr,
  string::String,
};

use futures::stream::{BoxStream, StreamExt};
use sqlparser::{
  ast::Statement,
  dialect::PostgreSqlDialect,
  parser::{Parser, ParserError},
};
use sqlx::{
  postgres::{PgConnectOptions, PgQueryResult, Postgres},
  types::Uuid,
  Column, Database, Either, Row, ValueRef,
};

use super::{vec_to_string, Value};

impl super::BuildConnectionOptions for sqlx::Postgres {
  fn build_connection_opts(
    args: crate::cli::Cli,
  ) -> color_eyre::eyre::Result<<Self::Connection as sqlx::Connection>::Options> {
    match args.connection_url {
      Some(url) => Ok(PgConnectOptions::from_str(&url)?),
      None => {
        let mut opts = PgConnectOptions::new();

        if let Some(user) = args.user {
          opts = opts.username(&user);
        } else {
          let mut user: String = String::new();
          print!("username: ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut user).unwrap();
          user = user.trim().to_string();
          if !user.is_empty() {
            opts = opts.username(&user);
          }
        }

        if let Some(password) = args.password {
          opts = opts.password(&password);
        } else {
          let mut password =
            rpassword::prompt_password(format!("password for user {}: ", opts.get_username())).unwrap();
          password = password.trim().to_string();
          if !password.is_empty() {
            opts = opts.password(&password);
          }
        }

        if let Some(host) = args.host {
          opts = opts.host(&host);
        } else {
          let mut host: String = String::new();
          print!("host (ex. localhost): ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut host).unwrap();
          host = host.trim().to_string();
          if !host.is_empty() {
            opts = opts.host(&host);
          }
        }

        if let Some(port) = args.port {
          opts = opts.port(port);
        } else {
          let mut port: String = String::new();
          print!("port (ex. 5432): ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut port).unwrap();
          port = port.trim().to_string();
          if !port.is_empty() {
            opts = opts.port(port.parse()?);
          }
        }

        if let Some(database) = args.database {
          opts = opts.database(&database);
        } else {
          let mut database: String = String::new();
          print!("database (ex. postgres): ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut database).unwrap();
          database = database.trim().to_string();
          if !database.is_empty() {
            opts = opts.database(&database);
          }
        }

        Ok(opts)
      },
    }
  }
}

impl super::HasRowsAffected for PgQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}

impl super::DatabaseQueries for Postgres {
  fn preview_tables_query() -> String {
    "select table_schema, table_name
      from information_schema.tables
      where table_schema != 'pg_catalog'
      and table_schema != 'information_schema'
      group by table_schema, table_name
      order by table_schema, table_name asc"
      .to_owned()
  }

  fn preview_rows_query(schema: &str, table: &str) -> String {
    format!("select * from \"{}\".\"{}\" limit 100", schema, table)
  }

  fn preview_columns_query(schema: &str, table: &str) -> String {
    format!(
      "select column_name, * from information_schema.columns where table_schema = '{}' and table_name = '{}'",
      schema, table
    )
  }

  fn preview_constraints_query(schema: &str, table: &str) -> String {
    format!(
      "select constraint_name, * from information_schema.table_constraints where table_schema = '{}' and table_name = '{}'",
      schema, table
    )
  }

  fn preview_indexes_query(schema: &str, table: &str) -> String {
    format!("select indexname, indexdef, * from pg_indexes where schemaname = '{}' and tablename = '{}'", schema, table)
  }

  fn preview_policies_query(schema: &str, table: &str) -> String {
    format!("select * from pg_policies where schemaname = '{}' and tablename = '{}'", schema, table)
  }
}

impl super::ValueParser for Postgres {
  // parsed based on https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html
  fn parse_value(row: &<Postgres as sqlx::Database>::Row, col: &<Postgres as sqlx::Database>::Column) -> Option<Value> {
    let col_type = col.type_info().to_string();
    if row.try_get_raw(col.ordinal()).is_ok_and(|v| v.is_null()) {
      return Some(Value { parse_error: false, string: "NULL".to_string(), is_null: true });
    }
    match col_type.to_uppercase().as_str() {
      "TIMESTAMPTZ" => {
        Some(
          row
            .try_get::<chrono::DateTime<chrono::Utc>, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TIMESTAMP" => {
        Some(
          row
            .try_get::<chrono::NaiveDateTime, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "DATE" => {
        Some(
          row
            .try_get::<chrono::NaiveDate, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TIME" => {
        Some(
          row
            .try_get::<chrono::NaiveTime, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "UUID" => {
        Some(
          row
            .try_get::<Uuid, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "INET" | "CIDR" => {
        Some(
          row
            .try_get::<std::net::IpAddr, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "JSON" | "JSONB" => {
        Some(
          row
            .try_get::<serde_json::Value, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "BOOL" => {
        Some(
          row
            .try_get::<bool, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "SMALLINT" | "SMALLSERIAL" | "INT2" => {
        Some(
          row
            .try_get::<i16, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "INT" | "SERIAL" | "INT4" => {
        Some(
          row
            .try_get::<i32, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "BIGINT" | "BIGSERIAL" | "INT8" => {
        Some(
          row
            .try_get::<i64, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "REAL" | "FLOAT4" => {
        Some(
          row
            .try_get::<f32, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "DOUBLE PRECISION" | "FLOAT8" => {
        Some(
          row
            .try_get::<f64, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TEXT" | "VARCHAR" | "NAME" | "CITEXT" | "BPCHAR" | "CHAR" => {
        Some(
          row
            .try_get::<String, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "BYTEA" => {
        Some(row.try_get::<Vec<u8>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| {
            Value {
              parse_error: false,
              string: received.iter().fold(String::new(), |mut output, b| {
                let _ = write!(output, "{b:02X}");
                output
              }),
              is_null: false,
            }
          },
        ))
      },
      "VOID" => Some(Value { parse_error: false, string: "".to_string(), is_null: false }),
      _ if col_type.to_uppercase().ends_with("[]") => {
        let array_type = col_type.to_uppercase().replace("[]", "");
        match array_type.as_str() {
          "TIMESTAMPTZ" => {
            Some(
              row
                .try_get::<Vec<chrono::DateTime<chrono::Utc>>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "TIMESTAMP" => {
            Some(
              row
                .try_get::<Vec<chrono::NaiveDateTime>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "DATE" => {
            Some(
              row
                .try_get::<Vec<chrono::NaiveDate>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "TIME" => {
            Some(
              row
                .try_get::<Vec<chrono::NaiveTime>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "UUID" => {
            Some(
              row
                .try_get::<Vec<Uuid>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "INET" | "CIDR" => {
            Some(
              row
                .try_get::<Vec<std::net::IpAddr>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "JSON" | "JSONB" => {
            Some(
              row
                .try_get::<Vec<serde_json::Value>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "BOOL" => {
            Some(
              row
                .try_get::<Vec<bool>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "SMALLINT" | "SMALLSERIAL" | "INT2" => {
            Some(
              row
                .try_get::<Vec<i16>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "INT" | "SERIAL" | "INT4" => {
            Some(
              row
                .try_get::<Vec<i32>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "BIGINT" | "BIGSERIAL" | "INT8" => {
            Some(
              row
                .try_get::<Vec<i64>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "REAL" | "FLOAT4" => {
            Some(
              row
                .try_get::<Vec<f32>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "DOUBLE PRECISION" | "FLOAT8" => {
            Some(
              row
                .try_get::<Vec<f64>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "TEXT" | "VARCHAR" | "NAME" | "CITEXT" | "BPCHAR" | "CHAR" => {
            Some(
              row
                .try_get::<Vec<String>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
          "BYTEA" => {
            Some(row.try_get::<Vec<u8>, usize>(col.ordinal()).map_or(
              Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
              |received| {
                Value {
                  parse_error: false,
                  string: received.iter().fold(String::new(), |mut output, b| {
                    let _ = write!(output, "{b:02X}");
                    output
                  }),
                  is_null: false,
                }
              },
            ))
          },
          _ => {
            // try to cast custom or other types to strings
            Some(
              row
                .try_get_unchecked::<Vec<String>, usize>(col.ordinal())
                .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
                  Value { parse_error: false, string: vec_to_string(received), is_null: false }
                }),
            )
          },
        }
      },
      _ => {
        // try to cast custom or other types to strings
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
  use std::sync::Arc;

  use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};

  use super::*;
  use crate::database::{get_execution_type, get_first_query, DbError, ExecutionType};

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

    let dialect = Box::new(PostgreSqlDialect {});

    for (input, expected_output) in test_cases {
      let result = get_first_query(input.to_string(), dialect.as_ref());
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
  fn test_execution_type_postgres() {
    let dialect = PostgreSqlDialect {};
    let test_cases = vec![
      ("DELETE FROM users WHERE id = 1", ExecutionType::Transaction),
      ("DROP TABLE users", ExecutionType::Confirm),
      ("UPDATE users SET name = 'John' WHERE id = 1", ExecutionType::Transaction),
      ("SELECT * FROM users", ExecutionType::Normal),
      ("INSERT INTO users (name) VALUES ('John')", ExecutionType::Normal),
      ("EXPLAIN ANALYZE DELETE FROM users WHERE id = 1", ExecutionType::Transaction),
      ("EXPLAIN ANALYZE DROP TABLE users", ExecutionType::Confirm),
      ("EXPLAIN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN ANALYZE SELECT * FROM users WHERE id = 1", ExecutionType::Normal),
    ];

    for (query, expected) in test_cases {
      let ast = Parser::parse_sql(&dialect, query).unwrap();
      let statement = ast[0].clone();
      assert_eq!(get_execution_type(statement, false), expected, "Failed for query: {}", query);
    }
  }
}
