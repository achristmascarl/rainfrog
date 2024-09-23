use super::Value;
use crate::generic_database::ValueParser;

impl ValueParser for MySql {
  fn parse_value(row: &Self::Row, col: &Self::Column) -> Option<Value> {
    // MySQL-specific parsing
    todo!()
  }
}
