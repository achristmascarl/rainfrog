// vim emulation for tui_textarea. based on:
// https://github.com/rhysd/tui-textarea/blob/main/examples/vim.rs
use std::fmt;

#[cfg(not(feature = "termux"))]
use arboard::Clipboard;
use color_eyre::eyre::Result;
use ratatui::{
  style::{Color, Modifier, Style},
  text::Line,
  widgets::{Block, Borders},
};
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{CursorMove, Input, Key, Scrolling, TextArea};

use crate::action::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
  #[default]
  Normal,
  Insert,
  Visual,
  Replace,
  Operator(char),
}

pub enum SelectionDirection {
  Forward,
  Backward,
  Neutral,
}

fn get_selection_direction(range: ((usize, usize), (usize, usize)), cursor: (usize, usize)) -> SelectionDirection {
  let (start, end) = range;
  if cursor == start && cursor == end {
    SelectionDirection::Neutral
  } else if cursor == end {
    SelectionDirection::Forward
  } else {
    SelectionDirection::Backward
  }
}

impl Mode {
  pub fn block<'a>(&self) -> Block<'a> {
    let help = match self {
      Self::Normal => "type i to enter insert mode, v to enter visual mode",
      Self::Insert => "type Esc to back to normal mode",
      Self::Visual => "type y to yank, type d to delete, type Esc to back to normal mode",
      Self::Replace => "type character to replace underlined",
      Self::Operator(_) => "move cursor to apply operator",
    };
    let title = format!(" {self} MODE ({help}) ");
    Block::default().borders(Borders::ALL).title_bottom(Line::from(title).right_aligned())
  }

  pub fn cursor_style(&self) -> Style {
    match self {
      Self::Normal => Style::default().fg(Color::Reset).add_modifier(Modifier::REVERSED),
      Self::Insert => Style::default().fg(Color::LightBlue).add_modifier(Modifier::SLOW_BLINK | Modifier::REVERSED),
      Self::Visual => Style::default().fg(Color::LightYellow).add_modifier(Modifier::REVERSED),
      Self::Replace => Style::default().fg(Color::LightMagenta).add_modifier(Modifier::UNDERLINED | Modifier::REVERSED),
      Self::Operator(_) => Style::default().fg(Color::LightGreen).add_modifier(Modifier::REVERSED),
    }
  }
}

impl fmt::Display for Mode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
    match self {
      Self::Normal => write!(f, "NORMAL"),
      Self::Insert => write!(f, "INSERT"),
      Self::Visual => write!(f, "VISUAL"),
      Self::Replace => write!(f, "REPLACE"),
      Self::Operator(c) => write!(f, "OPERATOR({c})"),
    }
  }
}

// How the Vim emulation state transitions
pub enum Transition {
  Nop,
  Mode(Mode),
  Pending(Input),
}

// State of Vim emulation
#[derive(Default, Clone)]
pub struct Vim {
  pub mode: Mode,
  pub pending: Input, // Pending input to handle a sequence with two keys like gg
  command_tx: Option<UnboundedSender<Action>>,
}

impl Vim {
  pub fn new(mode: Mode) -> Self {
    Self { mode, pending: Input::default(), command_tx: None }
  }

  pub fn with_pending(self, pending: Input) -> Self {
    Self { mode: self.mode, pending, command_tx: None }
  }

  pub fn register_action_handler(&mut self, tx: Option<UnboundedSender<Action>>) -> Result<()> {
    self.command_tx = tx;
    Ok(())
  }

  pub fn transition(&self, input: Input, textarea: &mut TextArea<'_>) -> Transition {
    if input.key == Key::Null {
      return Transition::Nop;
    }

    match self.mode {
      Mode::Normal | Mode::Visual | Mode::Operator(_) => {
        match input {
          Input { key: Key::Char('h'), .. } | Input { key: Key::Left, .. } => textarea.move_cursor(CursorMove::Back),
          Input { key: Key::Char('j'), .. } | Input { key: Key::Down, .. } => textarea.move_cursor(CursorMove::Down),
          Input { key: Key::Char('k'), .. } | Input { key: Key::Up, .. } => textarea.move_cursor(CursorMove::Up),
          Input { key: Key::Char('l'), .. } | Input { key: Key::Right, .. } => {
            textarea.move_cursor(CursorMove::Forward)
          },
          Input { key: Key::Char('w'), .. } => textarea.move_cursor(CursorMove::WordForward),
          Input { key: Key::Char('e'), ctrl: false, .. } if matches!(self.mode, Mode::Operator(_)) => {
            textarea.move_cursor(CursorMove::WordForward) // `e` behaves like `w` in operator-pending mode
          },
          Input { key: Key::Char('e'), ctrl: false, .. } => textarea.move_cursor(CursorMove::WordEnd),
          Input { key: Key::Char('b'), ctrl: false, .. } => textarea.move_cursor(CursorMove::WordBack),
          Input { key: Key::Char('^'), .. } => textarea.move_cursor(CursorMove::Head),
          Input { key: Key::Char('0'), .. } => textarea.move_cursor(CursorMove::Head),
          Input { key: Key::Char('$'), .. } => textarea.move_cursor(CursorMove::End),
          Input { key: Key::Char('D'), .. } => {
            textarea.delete_line_by_end();
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('C'), .. } => {
            textarea.delete_line_by_end();
            textarea.cancel_selection();
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('p'), .. } => {
            #[cfg(not(feature = "termux"))]
            {
              Clipboard::new().map_or_else(
                |e| log::error!("{e:?}"),
                |mut clipboard| {
                  clipboard.get_text().map_or_else(|e| log::error!("{e:?}"), |text| textarea.set_yank_text(text))
                },
              );
            }
            textarea.paste();
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('u'), ctrl: false, .. } => {
            textarea.undo();
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('r'), ctrl: true, .. } => {
            textarea.redo();
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('r'), ctrl: false, .. } => {
            return Transition::Mode(Mode::Replace);
          },
          Input { key: Key::Char('x'), .. } => {
            if !textarea.is_selecting() {
              textarea.start_selection();
            }
            if let Some(selection_range) = textarea.selection_range() {
              let selection_direction = get_selection_direction(selection_range, textarea.cursor());
              match selection_direction {
                SelectionDirection::Backward => {},
                _ => {
                  textarea.move_cursor(CursorMove::Forward); // Vim's forward text selection is inclusive
                },
              }
            }
            textarea.cut();
            self.send_copy_action_with_text(textarea.yank_text());
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('X'), .. } => {
            if self.mode == Mode::Visual {
              textarea.move_cursor(CursorMove::Head);
              textarea.start_selection();
              textarea.move_cursor(CursorMove::End);
            } else {
              textarea.start_selection();
              textarea.move_cursor(CursorMove::Back);
            }
            textarea.cut();
            self.send_copy_action_with_text(textarea.yank_text());
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('i'), .. } => {
            textarea.cancel_selection();
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('a'), ctrl: false, .. }
            if matches!(self.mode, Mode::Operator('d')) || matches!(self.mode, Mode::Operator('y')) =>
          {
            textarea.cancel_selection();
            textarea.move_cursor(CursorMove::Forward);
            textarea.move_cursor(CursorMove::WordBack);
            textarea.start_selection();
            return Transition::Nop;
          },
          Input { key: Key::Char('a'), .. } => {
            textarea.cancel_selection();
            textarea.move_cursor(CursorMove::Forward);
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('A'), .. } => {
            textarea.cancel_selection();
            textarea.move_cursor(CursorMove::End);
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('o'), .. } => {
            textarea.move_cursor(CursorMove::End);
            textarea.insert_newline();
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('O'), .. } => {
            textarea.move_cursor(CursorMove::Head);
            textarea.insert_newline();
            textarea.move_cursor(CursorMove::Up);
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('I'), .. } => {
            textarea.cancel_selection();
            textarea.move_cursor(CursorMove::Head);
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('e'), ctrl: true, .. } => textarea.scroll((1, 0)),
          Input { key: Key::Char('y'), ctrl: true, .. } => textarea.scroll((-1, 0)),
          Input { key: Key::Char('d'), ctrl: true, .. } => textarea.scroll(Scrolling::HalfPageDown),
          Input { key: Key::Char('u'), ctrl: true, .. } => textarea.scroll(Scrolling::HalfPageUp),
          Input { key: Key::Char('f'), ctrl: true, .. } | Input { key: Key::PageDown, .. } => {
            textarea.scroll(Scrolling::PageDown)
          },
          Input { key: Key::Char('b'), ctrl: true, .. } | Input { key: Key::PageUp, .. } => {
            textarea.scroll(Scrolling::PageUp)
          },
          Input { key: Key::Char('v'), ctrl: false, .. } if self.mode == Mode::Normal => {
            textarea.start_selection();
            return Transition::Mode(Mode::Visual);
          },
          Input { key: Key::Char('V'), ctrl: false, .. } if self.mode == Mode::Normal => {
            textarea.move_cursor(CursorMove::Head);
            textarea.start_selection();
            textarea.move_cursor(CursorMove::End);
            return Transition::Mode(Mode::Visual);
          },
          Input { key: Key::Esc, .. }
          | Input { key: Key::Char('c'), ctrl: true, .. }
          | Input { key: Key::Char('v'), ctrl: false, .. }
            if self.mode == Mode::Visual =>
          {
            textarea.cancel_selection();
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('g'), ctrl: false, .. }
            if matches!(self.pending, Input { key: Key::Char('g'), ctrl: false, .. }) =>
          {
            textarea.move_cursor(CursorMove::Top)
          },
          Input { key: Key::Char('G'), ctrl: false, .. } => textarea.move_cursor(CursorMove::Bottom),
          Input { key: Key::Char(c), ctrl: false, .. } if self.mode == Mode::Operator(c) => {
            // Handle yy, dd, cc. (This is not strictly the same behavior as Vim)
            textarea.move_cursor(CursorMove::Head);
            textarea.start_selection();
            let cursor = textarea.cursor();
            textarea.move_cursor(CursorMove::Down);
            if cursor == textarea.cursor() {
              textarea.move_cursor(CursorMove::End); // At the last line, move to end of the line instead
            }
          },
          Input { key: Key::Char(op @ ('y' | 'd' | 'c')), ctrl: false, .. } if self.mode == Mode::Normal => {
            textarea.start_selection();
            return Transition::Mode(Mode::Operator(op));
          },
          Input { key: Key::Char('y'), ctrl: false, .. } if self.mode == Mode::Visual => {
            if let Some(selection_range) = textarea.selection_range() {
              let selection_direction = get_selection_direction(selection_range, textarea.cursor());
              match selection_direction {
                SelectionDirection::Backward => {},
                _ => {
                  textarea.move_cursor(CursorMove::Forward); // Vim's forward text selection is inclusive
                },
              }
            }
            textarea.copy();
            self.send_copy_action_with_text(textarea.yank_text());
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('d'), ctrl: false, .. } if self.mode == Mode::Visual => {
            if let Some(selection_range) = textarea.selection_range() {
              let selection_direction = get_selection_direction(selection_range, textarea.cursor());
              match selection_direction {
                SelectionDirection::Backward => {},
                _ => {
                  textarea.move_cursor(CursorMove::Forward); // Vim's forward text selection is inclusive
                },
              }
            }
            textarea.cut();
            return Transition::Mode(Mode::Normal);
          },
          Input { key: Key::Char('c'), ctrl: false, .. } if self.mode == Mode::Visual => {
            if let Some(selection_range) = textarea.selection_range() {
              let selection_direction = get_selection_direction(selection_range, textarea.cursor());
              match selection_direction {
                SelectionDirection::Backward => {},
                _ => {
                  textarea.move_cursor(CursorMove::Forward); // Vim's forward text selection is inclusive
                },
              }
            }
            textarea.cut();
            self.send_copy_action_with_text(textarea.yank_text());
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Char('S'), ctrl: false, .. } => {
            textarea.move_cursor(CursorMove::Head);
            textarea.start_selection();
            textarea.move_cursor(CursorMove::End);
            textarea.cut();
            self.send_copy_action_with_text(textarea.yank_text());
            return Transition::Mode(Mode::Insert);
          },
          Input { key: Key::Esc, .. } => {
            textarea.cancel_selection();
            return Transition::Mode(Mode::Normal);
          },
          input => return Transition::Pending(input),
        }

        // Handle the pending operator
        match self.mode {
          Mode::Operator('y') => {
            textarea.copy();
            self.send_copy_action_with_text(textarea.yank_text());
            Transition::Mode(Mode::Normal)
          },
          Mode::Operator('d') => {
            textarea.cut();
            Transition::Mode(Mode::Normal)
          },
          Mode::Operator('c') => {
            textarea.cut();
            self.send_copy_action_with_text(textarea.yank_text());
            Transition::Mode(Mode::Insert)
          },
          _ => Transition::Nop,
        }
      },
      Mode::Insert => {
        match input {
          Input { key: Key::Esc, .. } | Input { key: Key::Char('c'), ctrl: true, .. } => Transition::Mode(Mode::Normal),
          input => {
            textarea.input(input); // Use default key mappings in insert mode
            Transition::Mode(Mode::Insert)
          },
        }
      },
      Mode::Replace => match input {
        Input { key: Key::Esc, .. } | Input { key: Key::Char('c'), ctrl: true, .. } => Transition::Mode(Mode::Normal),
        input => {
          textarea.delete_str(1);
          textarea.input(input);
          Transition::Mode(Mode::Normal)
        },
      },
    }
  }

  fn send_copy_action_with_text(&self, text: String) {
    if let Some(sender) = &self.command_tx {
      sender.send(Action::CopyData(text)).map_or_else(|e| log::error!("{e:?}"), |_| {});
    }
  }
}
