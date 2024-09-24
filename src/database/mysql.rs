use super::Value;
use crate::generic_database::ValueParser;

impl super::HasRowsAffected for MySqlQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}

impl ValueParser for MySql {
  fn parse_value(row: &Self::Row, col: &Self::Column) -> Option<Value> {
    // MySQL-specific parsing
    todo!()
  }
}
