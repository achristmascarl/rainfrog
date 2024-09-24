impl super::HasRowsAffected for SqliteQueryResult {
  fn rows_affected(&self) -> u64 {
    self.rows_affected()
  }
}

impl ValueParser for Sqlite {
  fn parse_value(row: &Self::Row, col: &Self::Column) -> Option<Value> {
    // SQLite-specific parsing
    todo!()
  }
}
