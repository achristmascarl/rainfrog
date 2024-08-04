use ratatui::{layout::*, prelude};

pub fn center(area: Rect, horizontal: Constraint, vertical: Constraint) -> Rect {
  let [area] = Layout::horizontal([horizontal]).flex(Flex::Center).areas(area);
  let [area] = Layout::vertical([vertical]).flex(Flex::Center).areas(area);
  area
}
