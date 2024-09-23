use std::collections::HashMap;

use futures::stream::{BoxStream, StreamExt};
use sqlparser::{
  ast::Statement,
  dialect::{Dialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect},
  keywords,
  parser::{Parser, ParserError},
};
use sqlx::{
  mysql::{MySql, MySqlColumn, MySqlQueryResult, MySqlRow},
  pool::PoolOptions,
  postgres::{PgColumn, PgQueryResult, PgRow, Postgres},
  sqlite::{Sqlite, SqliteColumn, SqliteQueryResult, SqliteRow},
  Column, Connection, Database, Either, Error, Executor, Pool, Row, Transaction,
};

mod postgresql;

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
pub type DbPool<DB> = Pool<DB>;
pub type DbError = Either<Error, ParserError>;

pub trait HasRowsAffected {
  fn rows_affected(&self) -> u64;
}

// Implement for PostgreSQL
impl HasRowsAffected for PgQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}

// Implement for MySQL
impl HasRowsAffected for MySqlQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}

// Implement for SQLite
impl HasRowsAffected for SqliteQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}

pub async fn init_pool<DB: Database>(opts: <DB::Connection as Connection>::Options) -> Result<Pool<DB>, Error> {
  PoolOptions::new().max_connections(5).connect_with(opts).await
}

pub async fn query<DB>(query: String, pool: &Pool<DB>) -> Result<Rows, DbError>
where
  DB: Database + ValueParser,
  DB::QueryResult: HasRowsAffected,
  for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
  // get the db type from
  let dialect = get_dialect(DB::NAME);
  let first_query = get_first_query(query, dialect.as_ref());
  match first_query {
    Ok((first_query, _)) => {
      let stream = sqlx::raw_sql(&first_query).fetch_many(pool);
      query_stream::<DB>(stream).await
    },
    Err(e) => Err(e),
  }
}

pub fn get_first_query(query: String, dialect: &dyn Dialect) -> Result<(String, Statement), DbError> {
  let ast = Parser::parse_sql(dialect, &query);
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

pub fn get_dialect(db_type: &str) -> Box<dyn Dialect + Send + Sync> {
  match db_type {
    "postgres" => Box::new(PostgreSqlDialect {}),
    "mysql" => Box::new(MySqlDialect {}),
    _ => Box::new(SQLiteDialect {}),
  }
}

pub trait ValueParser: Database {
  fn parse_value(row: &Self::Row, col: &Self::Column) -> Option<Value>;
}

pub fn row_to_vec<DB: Database + ValueParser>(row: &DB::Row) -> Vec<String> {
  row.columns().iter().map(|col| DB::parse_value(row, col).unwrap().string).collect()
}

pub fn row_to_json<DB: Database + ValueParser>(row: &DB::Row) -> HashMap<String, String> {
  let mut result = HashMap::new();
  for col in row.columns() {
    let value = match DB::parse_value(row, col) {
      Some(v) => v.string,
      _ => "[ unsupported ]".to_string(),
    };
    result.insert(col.name().to_string(), value);
  }

  result
}

pub fn get_headers<DB: Database + ValueParser>(row: &DB::Row) -> Headers {
  row
    .columns()
    .iter()
    .map(|col| Header { name: col.name().to_string(), type_name: col.type_info().to_string() })
    .collect()
}

pub async fn query_stream<'a, DB>(
  mut stream: BoxStream<'_, Result<Either<DB::QueryResult, DB::Row>, Error>>,
) -> Result<Rows, DbError>
where
  DB: Database + ValueParser,
  DB::QueryResult: HasRowsAffected,
{
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
        query_rows.push(row_to_vec::<DB>(&row));
        if headers.is_empty() {
          headers = get_headers::<DB>(&row);
        }
      },
      Some(Err(e)) => return Err(Either::Left(e)),
      None => return Err(Either::Left(Error::Protocol("Results stream empty".to_owned()))),
    };
  }
  Ok(Rows { rows_affected: query_rows_affected, headers, rows: query_rows })
}

pub async fn query_with_tx<'a, DB>(
  mut tx: Transaction<'_, DB>,
  query: String,
) -> (Result<Either<u64, Rows>, DbError>, Transaction<'_, DB>)
where
  DB: Database + ValueParser,
  DB::QueryResult: HasRowsAffected,
  for<'c> <DB as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, DB>,
  for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
  let dialect = get_dialect(DB::NAME);
  let first_query = get_first_query(query, dialect.as_ref());
  match first_query {
    Ok((first_query, statement_type)) => {
      match statement_type {
        Statement::Explain { .. } => {
          let stream = sqlx::raw_sql(&first_query).fetch_many(&mut *tx);
          let result = query_stream::<DB>(stream).await;
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

pub fn vec_to_string<T: std::string::ToString>(vec: Vec<T>) -> String {
  let mut content = String::new();
  for (i, elem) in vec.iter().enumerate() {
    content.push_str(&elem.to_string());
    if i != vec.len() - 1 {
      content.push_str(", ");
    }
  }
  "{ ".to_owned() + &*content + &*" }".to_owned()
}

pub fn get_keywords() -> Vec<String> {
  keywords::ALL_KEYWORDS.iter().map(|k| k.to_string()).collect()
}
