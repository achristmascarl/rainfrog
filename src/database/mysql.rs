use std::{
  fmt::Write,
  io::{self, Write as _},
  str::FromStr,
};

use serde_json;
use sqlparser::ast::Statement;
use sqlx::{
  mysql::{MySql, MySqlConnectOptions, MySqlQueryResult},
  Column, Database, Row, ValueRef,
};

use super::{vec_to_string, Value};

impl super::HasRowsAffected for MySqlQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}

impl super::BuildConnectionOptions for MySql {
  fn build_connection_opts(
    args: crate::cli::Cli,
  ) -> color_eyre::eyre::Result<<Self::Connection as sqlx::Connection>::Options> {
    match args.connection_url {
      Some(url) => Ok(MySqlConnectOptions::from_str(&url)?),
      None => {
        let mut opts = MySqlConnectOptions::new();

        // Username
        if let Some(user) = args.user {
          opts = opts.username(&user);
        } else {
          let mut user = String::new();
          print!("username: ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut user)?;
          let user = user.trim();
          if !user.is_empty() {
            opts = opts.username(user);
          }
        }

        // Password
        if let Some(password) = args.password {
          opts = opts.password(&password);
        } else {
          let password = rpassword::prompt_password(format!("password for user {}: ", opts.get_username())).unwrap();
          let password = password.trim();
          if !password.is_empty() {
            opts = opts.password(password);
          }
        }

        // Host
        if let Some(host) = args.host {
          opts = opts.host(&host);
        } else {
          let mut host = String::new();
          print!("host (ex. localhost): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut host)?;
          let host = host.trim();
          if !host.is_empty() {
            opts = opts.host(host);
          }
        }

        // Port
        if let Some(port) = args.port {
          opts = opts.port(port);
        } else {
          let mut port = String::new();
          print!("port (ex. 3306): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut port)?;
          let port = port.trim();
          if !port.is_empty() {
            opts = opts.port(port.parse()?);
          }
        }

        // Database
        if let Some(database) = args.database {
          opts = opts.database(&database);
        } else {
          let mut database = String::new();
          print!("database (ex. mydb): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim();
          if !database.is_empty() {
            opts = opts.database(database);
          }
        }

        Ok(opts)
      },
    }
  }
}

impl super::DatabaseQueries for MySql {
  fn preview_tables_query() -> String {
    "select table_schema as table_schema, table_name as table_name
      from information_schema.tables
      where table_schema not in ('mysql', 'information_schema', 'performance_schema', 'sys')
      order by table_schema, table_name asc"
      .to_owned()
  }

  fn preview_rows_query(schema: &str, table: &str) -> String {
    format!("select * from `{}`.`{}` limit 100", schema, table)
  }

  fn preview_columns_query(schema: &str, table: &str) -> String {
    format!(
      "select column_name, data_type, is_nullable, column_default, extra, column_comment
        from information_schema.columns
        where table_schema = '{}' and table_name = '{}'
        order by ordinal_position",
      schema, table
    )
  }

  fn preview_constraints_query(schema: &str, table: &str) -> String {
    format!(
      "select constraint_name, constraint_type, enforced,
        group_concat(column_name order by ordinal_position) as column_names
        from information_schema.table_constraints
        join information_schema.key_column_usage using (constraint_schema, constraint_name, table_schema, table_name)
        where table_schema = '{}' and table_name = '{}'
        group by constraint_name, constraint_type, enforced
        order by constraint_type, constraint_name",
      schema, table
    )
  }

  fn preview_indexes_query(schema: &str, table: &str) -> String {
    format!(
      "select index_name, column_name, non_unique, seq_in_index, index_type
        from information_schema.statistics
        where table_schema = '{}' and table_name = '{}'
        order by index_name, seq_in_index",
      schema, table
    )
  }

  fn preview_policies_query(_schema: &str, _table: &str) -> String {
    "select 'MySQL does not support row-level security policies' as message".to_owned()
  }
}

impl super::ValueParser for MySql {
  fn parse_value(row: &<MySql as sqlx::Database>::Row, col: &<MySql as sqlx::Database>::Column) -> Option<Value> {
    let col_type = col.type_info().to_string();
    if row.try_get_raw(col.ordinal()).is_ok_and(|v| v.is_null()) {
      return Some(Value { parse_error: false, string: "NULL".to_string(), is_null: true });
    }
    match col_type.to_uppercase().as_str() {
      "TINYINT(1)" | "BOOLEAN" | "BOOL" => {
        Some(
          row
            .try_get::<bool, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TINYINT" => {
        Some(
          row
            .try_get::<i8, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "SMALLINT" => {
        Some(
          row
            .try_get::<i16, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "INT" => {
        Some(
          row
            .try_get::<i32, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "BIGINT" => {
        Some(
          row
            .try_get::<i64, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TINYINT UNSIGNED" => {
        Some(
          row
            .try_get::<u8, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "SMALLINT UNSIGNED" => {
        Some(
          row
            .try_get::<u16, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "INT UNSIGNED" => {
        Some(
          row
            .try_get::<u32, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "BIGINT UNSIGNED" => {
        Some(
          row
            .try_get::<u64, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "FLOAT" => {
        Some(
          row
            .try_get::<f32, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "DOUBLE" => {
        Some(
          row
            .try_get::<f64, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "VARCHAR" | "CHAR" | "TEXT" | "BINARY" => {
        Some(
          row
            .try_get::<String, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "VARBINARY" | "BLOB" => {
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
      "INET4" | "INET6" => {
        Some(
          row
            .try_get::<std::net::IpAddr, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TIME" => {
        Some(
          row.try_get::<chrono::NaiveTime, usize>(col.ordinal()).map_or(
            row
              .try_get::<chrono::TimeDelta, usize>(col.ordinal())
              .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: true }, |received| {
                Value { parse_error: false, string: received.to_string(), is_null: false }
              }),
            |received| Value { parse_error: false, string: received.to_string(), is_null: false },
          ),
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
      "DATETIME" => {
        Some(
          row
            .try_get::<chrono::NaiveDateTime, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "TIMESTAMP" => {
        Some(
          row
            .try_get::<chrono::DateTime<chrono::Utc>, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "JSON" => {
        Some(
          row
            .try_get::<serde_json::Value, usize>(col.ordinal())
            .map_or(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }, |received| {
              Value { parse_error: false, string: received.to_string(), is_null: false }
            }),
        )
      },
      "GEOMETRY" => {
        // TODO: would have to resort to geozero to parse WKB
        Some(Value { parse_error: true, string: "_TODO_".to_owned(), is_null: false })
      },
      _ => {
        // Try to cast custom or other types to strings
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
    dialect::MySqlDialect,
    parser::{Parser, ParserError},
  };

  use super::*;
  use crate::database::{get_execution_type, get_first_query, DbError, ExecutionType};

  #[test]
  fn test_get_first_query_mysql() {
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
        "select * from `users`",
        Ok(("SELECT * FROM `users`".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
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

    let dialect = Box::new(MySqlDialect {});

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
  fn test_execution_type_mysql() {
    let dialect = MySqlDialect {};
    let test_cases = vec![
      ("DELETE FROM users WHERE id = 1", ExecutionType::Transaction),
      ("DROP TABLE users", ExecutionType::Confirm),
      ("UPDATE users SET name = 'John' WHERE id = 1", ExecutionType::Transaction),
      ("SELECT * FROM users", ExecutionType::Normal),
      ("INSERT INTO users (name) VALUES ('John')", ExecutionType::Normal),
      ("EXPLAIN DELETE FROM users WHERE id = 1", ExecutionType::Normal),
      ("EXPLAIN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN ANALYZE UPDATE users SET name = 'John' WHERE id = 1", ExecutionType::Transaction),
      ("EXPLAIN ANALYZE DROP TABLE users", ExecutionType::Confirm),
    ];

    for (query, expected) in test_cases {
      let ast = Parser::parse_sql(&dialect, query).unwrap();
      let statement = ast[0].clone();
      assert_eq!(get_execution_type(statement, false), expected, "Failed for query: {}", query);
    }
  }
}
