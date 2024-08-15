use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::{prelude::*, widgets::*};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{Input, Key, Scrolling, TextArea};

use super::{Component, Frame};
use crate::{
  action::{Action, MenuPreview},
  app::{App, AppState},
  config::{Config, KeyBindings},
  focus::Focus,
  tui::Event,
};

#[derive(Default)]
pub struct History {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  list_state: ListState,
}

impl History {
  pub fn new() -> Self {
    History { command_tx: None, config: Config::default(), list_state: ListState::default() }
  }
}

impl Component for History {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_mouse_events(&mut self, mouse: MouseEvent, app_state: &AppState) -> Result<Option<Action>> {
    if app_state.focus != Focus::History {
      return Ok(None);
    }
    // match mouse.kind {
    //   MouseEventKind::ScrollDown => {
    //     self.textarea.scroll((1, 0));
    //   },
    //   MouseEventKind::ScrollUp => {
    //     self.textarea.scroll((-1, 0));
    //   },
    //   MouseEventKind::ScrollLeft => {
    //     self.transition_vim_state(Input { key: Key::Char('h'), ctrl: false, alt: false, shift: false })?;
    //   },
    //   MouseEventKind::ScrollRight => {
    //     self.transition_vim_state(Input { key: Key::Char('j'), ctrl: false, alt: false, shift: false })?;
    //   },
    //   _ => {},
    // };
    Ok(None)
  }

  fn handle_events(
    &mut self,
    event: Option<Event>,
    last_tick_key_events: Vec<KeyEvent>,
    app_state: &AppState,
  ) -> Result<Option<Action>> {
    if app_state.focus != Focus::History {
      return Ok(None);
    }
    // if let Some(Event::Paste(text)) = event {
    //   self.textarea.insert_str(text);
    // } else if let Some(Event::Mouse(event)) = event {
    //   self.handle_mouse_events(event, app_state).unwrap();
    // } else if let Some(Event::Key(key)) = event {
    //   if app_state.query_task.is_none() {
    //     let input = Input::from(key);
    //     self.transition_vim_state(input)?;
    //   }
    // };
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    let focused = app_state.focus == Focus::History;
    let block = Block::default().borders(Borders::ALL).border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });
    let block_margin = block.inner(area).inner(Margin { vertical: 1, horizontal: 0 });

    let list = List::default()
      .items(vec!["blah", "blah2"])
      .block(block)
      .highlight_style(Style::default().fg(if focused { Color::Green } else { Color::Gray }).reversed());

    let paragraph = Paragraph::new("history").block(block);
    f.render_widget(paragraph, area);
    f.render_stateful_widget(list, layout[layout_index], &mut self.list_state);
    let vertical_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).symbols(scrollbar::VERTICAL).style(
      if focused && !self.search_focused && self.menu_focus == MenuFocus::Tables {
        Style::default().fg(Color::Green)
      } else {
        Style::default()
      },
    );
    let mut vertical_scrollbar_state =
      ScrollbarState::new(table_length.saturating_sub(available_height)).position(self.list_state.offset());
    f.render_stateful_widget(vertical_scrollbar, block_margin, &mut vertical_scrollbar_state);
    Ok(())
  }
}
