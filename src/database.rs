use std::{collections::HashMap, fmt::Write, string::String};

use sqlx::{
  postgres::{PgColumn, PgPool, PgPoolOptions, PgQueryResult, PgRow, PgValueRef, Postgres},
  types::Uuid,
  Column, Database, Error, Pool, Row, ValueRef,
};

pub type Rows = Vec<PgRow>;
pub type DbPool = PgPool;
pub type DbError = Error;

pub async fn init_pool(url: String) -> Result<PgPool, Error> {
  PgPoolOptions::new().max_connections(5).connect(&url).await
}

pub async fn query(query: String, pool: &PgPool) -> Result<Rows, Error> {
  sqlx::query(&query).fetch_all(pool).await
}

// courtesy of https://stackoverflow.com/questions/72901680/convert-pgrow-value-of-unknown-type-to-a-string
pub fn row_to_json(row: &PgRow) -> HashMap<String, String> {
  let mut result = HashMap::new();
  for col in row.columns() {
    let value = row.try_get_raw(col.ordinal()).unwrap();
    let value = match value.is_null() {
      true => "NULL".to_string(),
      false => {
        match value.type_info().to_string().as_str() {
          "UUID" => {
            Uuid::parse_str(&value.as_bytes().unwrap().to_vec().iter().fold(String::new(), |mut output, b| {
              let _ = write!(output, "{b:02X}");
              output
            }))
            .unwrap()
            .to_string()
          },
          _ => value.type_info().to_string(),
        }
      },
    };
    result.insert(col.name().to_string(), value);
  }

  result
}
