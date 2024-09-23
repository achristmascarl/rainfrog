
impl ValueParser for Sqlite {
  fn parse_value(row: &Self::Row, col: &Self::Column) -> Option<Value> {
    // SQLite-specific parsing
    todo!()
  }
}