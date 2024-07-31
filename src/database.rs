use std::{collections::HashMap, fmt::Write, string::String};

use sqlx::{
  postgres::{PgColumn, PgPool, PgPoolOptions, PgQueryResult, PgRow, PgTypeInfo, PgTypeKind, PgValueRef, Postgres},
  types::Uuid,
  Column, Database, Error, Pool, Row, ValueRef,
};

pub struct Header {
  pub name: String,
  pub type_name: String,
}

pub struct Value {
  pub is_null: bool,
  pub string: String,
}

pub type Rows = Vec<PgRow>;
pub type Headers = Vec<Header>;
pub type DbPool = PgPool;
pub type DbError = Error;

pub async fn init_pool(url: String) -> Result<PgPool, Error> {
  PgPoolOptions::new().max_connections(5).connect(&url).await
}

pub async fn query(query: String, pool: &PgPool) -> Result<Rows, Error> {
  sqlx::query(&query).fetch_all(pool).await
}

pub fn get_headers(rows: &Rows) -> Headers {
  match rows.len() {
    0 => vec![],
    _ => {
      rows[0]
        .columns()
        .iter()
        .map(|col| Header { name: col.name().to_string(), type_name: col.type_info().to_string() })
        .collect()
    },
  }
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
