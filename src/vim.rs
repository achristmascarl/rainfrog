// vim emulation for tui_textarea. based on:
// https://github.com/rhysd/tui-textarea/blob/main/examples/vim.rs
use std::{env, fmt, fs, io, io::BufRead};

#[cfg(not(feature = "termux"))]
use arboard::Clipboard;
use color_eyre::eyre::Result;
use crossterm::{
  event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent},
  terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use rat_text::{
  event::{
    crossterm::modifiers::{ALT, CONTROL, NONE, SHIFT},
    HandleEvent,
  },
  text_area::TextAreaState,
  TextRange,
};
use ratatui::{
  backend::CrosstermBackend,
  style::{Color, Modifier, Style, Stylize},
  text::Line,
  widgets::{Block, Borders},
  Terminal,
};
use tokio::sync::mpsc::UnboundedSender;

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
    let title = format!(" {} MODE ({}) ", self, help);
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
      Self::Operator(c) => write!(f, "OPERATOR({})", c),
    }
  }
}

// How the Vim emulation state transitions
pub enum Transition {
  Nop,
  Mode(Mode),
  Pending(KeyEvent),
}

// State of Vim emulation
#[derive(Default, Clone)]
pub struct Vim {
  pub mode: Mode,
  pub pending: Option<KeyEvent>, // Pending input to handle a sequence with two keys like gg
  command_tx: Option<UnboundedSender<Action>>,
}

impl Vim {
  pub fn new(mode: Mode) -> Self {
    Self { mode, pending: None, command_tx: None }
  }

  pub fn with_pending(self, pending: KeyEvent) -> Self {
    Self { mode: self.mode, pending: Some(pending), command_tx: None }
  }

  pub fn register_action_handler(&mut self, tx: Option<UnboundedSender<Action>>) -> Result<()> {
    self.command_tx = tx;
    Ok(())
  }

  pub fn transition(&self, input: KeyEvent, textarea: &mut TextAreaState) -> Transition {
    let extend_selection = self.mode != Mode::Normal;
    match self.mode {
      Mode::Normal | Mode::Visual | Mode::Operator(_) => {
        match input {
          KeyEvent { code: KeyCode::Char('h'), .. } | KeyEvent { code: KeyCode::Left, .. } => {
            textarea.move_left(1, extend_selection);
          },
          KeyEvent { code: KeyCode::Char('j'), .. } | KeyEvent { code: KeyCode::Down, .. } => {
            textarea.move_down(1, extend_selection);
          },
          KeyEvent { code: KeyCode::Char('k'), .. } | KeyEvent { code: KeyCode::Up, .. } => {
            textarea.move_up(1, extend_selection);
          },
          KeyEvent { code: KeyCode::Char('l'), .. } | KeyEvent { code: KeyCode::Right, .. } => {
            textarea.move_right(1, extend_selection);
          },
          KeyEvent { code: KeyCode::Char('w'), .. } => {
            textarea.move_to_next_word(extend_selection);
          },
          KeyEvent { code: KeyCode::Char('e'), modifiers: NONE, .. } if matches!(self.mode, Mode::Operator(_)) => {
            textarea.move_to_next_word(extend_selection); // `e` behaves like `w` in operator-pending mode
          },
          KeyEvent { code: KeyCode::Char('e'), modifiers: NONE, .. } => {
            textarea.next_word_end(textarea.cursor());
          },
          KeyEvent { code: KeyCode::Char('b'), modifiers: NONE, .. } => {
            textarea.prev_word_start(textarea.cursor());
          },
          KeyEvent { code: KeyCode::Char('^'), .. } => {
            textarea.move_to_line_start(extend_selection);
          },
          KeyEvent { code: KeyCode::Char('0'), .. } => {
            textarea.move_to_line_start(extend_selection);
          },
          KeyEvent { code: KeyCode::Char('$'), .. } => {
            textarea.move_to_line_end(extend_selection);
          },
          KeyEvent { code: KeyCode::Char('D'), .. } => {
            textarea.move_to_line_end(true);
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.delete_range(textarea.selection());
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('C'), .. } => {
            textarea.move_to_line_end(true);
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.delete_range(textarea.selection());
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('p'), .. } => {
            textarea.delete_range(textarea.selection());
            #[cfg(not(feature = "termux"))]
            {
              Clipboard::new().map_or_else(
                |e| log::error!("{e:?}"),
                |mut clipboard| {
                  clipboard.get_text().map_or_else(
                    |e| log::error!("{e:?}"),
                    |text| {
                      textarea.insert_str(text);
                    },
                  );
                },
              );
            }
            #[cfg(feature = "termux")]
            {
              textarea.paste_from_clip();
            }
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('u'), modifiers: NONE, .. } => {
            textarea.undo();
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('r'), modifiers: CONTROL, .. } => {
            textarea.redo();
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('r'), modifiers: NONE, .. } => {
            return Transition::Mode(Mode::Replace);
          },
          KeyEvent { code: KeyCode::Char('x'), .. } => {
            if !textarea.has_selection() {
              textarea.move_right(1, true);
            }
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.cut_to_clip();
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('X'), .. } => {
            if self.mode == Mode::Visual {
              textarea.move_to_line_start(false);
              textarea.move_to_line_end(true);
            } else {
              textarea.move_left(1, true);
            }
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.cut_to_clip();
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('i'), .. } => {
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('a'), modifiers: NONE, .. }
            if matches!(self.mode, Mode::Operator('d')) || matches!(self.mode, Mode::Operator('y')) =>
          {
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            textarea.move_right(1, false);
            textarea.word_start(textarea.cursor());
            return Transition::Nop;
          },
          KeyEvent { code: KeyCode::Char('a'), .. } => {
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            textarea.move_right(1, false);
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('A'), .. } => {
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            textarea.move_to_line_end(false);
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('o'), .. } => {
            textarea.move_to_line_end(false);
            textarea.insert_newline();
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('O'), .. } => {
            textarea.move_to_line_start(false);
            textarea.insert_newline();
            textarea.move_up(1, false);
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('I'), .. } => {
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            textarea.move_to_line_start(false);
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('e'), modifiers: CONTROL, .. } => {
            textarea.scroll_down(1);
          },
          KeyEvent { code: KeyCode::Char('y'), modifiers: CONTROL, .. } => {
            textarea.scroll_up(1);
          },
          KeyEvent { code: KeyCode::Char('d'), modifiers: CONTROL, .. } => {
            textarea.scroll_down(textarea.vertical_page().saturating_div(2));
          },
          KeyEvent { code: KeyCode::Char('u'), modifiers: CONTROL, .. } => {
            textarea.scroll_up(textarea.vertical_page().saturating_div(2));
          },
          KeyEvent { code: KeyCode::Char('f'), modifiers: CONTROL, .. } | KeyEvent { code: KeyCode::PageDown, .. } => {
            textarea.scroll_down(textarea.vertical_page());
          },
          KeyEvent { code: KeyCode::Char('b'), modifiers: CONTROL, .. } | KeyEvent { code: KeyCode::PageUp, .. } => {
            textarea.scroll_up(textarea.vertical_page());
          },
          KeyEvent { code: KeyCode::Char('v'), modifiers: NONE, .. } if self.mode == Mode::Normal => {
            textarea.move_right(1, true);
            return Transition::Mode(Mode::Visual);
          },
          KeyEvent { code: KeyCode::Char('V'), modifiers: NONE, .. } if self.mode == Mode::Normal => {
            textarea.move_to_line_start(false);
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            textarea.move_to_line_end(true);
            return Transition::Mode(Mode::Visual);
          },
          KeyEvent { code: KeyCode::Esc, .. }
          | KeyEvent { code: KeyCode::Char('c'), modifiers: CONTROL, .. }
          | KeyEvent { code: KeyCode::Char('v'), modifiers: NONE, .. }
            if self.mode == Mode::Visual =>
          {
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('g'), modifiers: NONE, .. }
            if matches!(self.pending, Some(KeyEvent { code: KeyCode::Char('g'), modifiers: NONE, .. })) =>
          {
            textarea.move_to_start(extend_selection);
          },
          KeyEvent { code: KeyCode::Char('G'), modifiers: NONE, .. } => {
            textarea.move_to_end(extend_selection);
          },
          KeyEvent { code: KeyCode::Char(c), modifiers: NONE, .. } if self.mode == Mode::Operator(c) => {
            // Handle yy, dd, cc. (This is not strictly the same behavior as Vim)
            textarea.move_to_line_start(false);
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            textarea.move_to_line_end(true);
          },
          KeyEvent { code: KeyCode::Char(op @ ('y' | 'd' | 'c')), modifiers: NONE, .. }
            if self.mode == Mode::Normal =>
          {
            return Transition::Mode(Mode::Operator(op));
          },
          KeyEvent { code: KeyCode::Char('y'), modifiers: NONE, .. } if self.mode == Mode::Visual => {
            if textarea.has_selection() {
              self.send_copy_action_with_text(textarea.selected_text().to_string());
              textarea.copy_to_clip();
            }
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('d'), modifiers: NONE, .. } if self.mode == Mode::Visual => {
            if textarea.has_selection() {
              textarea.cut_to_clip();
            }
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Char('c'), modifiers: NONE, .. } if self.mode == Mode::Visual => {
            if textarea.has_selection() {
              self.send_copy_action_with_text(textarea.selected_text().to_string());
              textarea.cut_to_clip();
            }
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Char('S'), modifiers: NONE, .. } => {
            textarea.move_to_line_start(false);
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            textarea.move_to_line_end(true);
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.cut_to_clip();
            return Transition::Mode(Mode::Insert);
          },
          KeyEvent { code: KeyCode::Esc, .. } => {
            textarea.set_selection(textarea.cursor(), textarea.cursor());
            return Transition::Mode(Mode::Normal);
          },
          input => return Transition::Pending(input),
        }

        // Handle the pending operator
        return match self.mode {
          Mode::Operator('y') => {
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.copy_to_clip();
            Transition::Mode(Mode::Normal)
          },
          Mode::Operator('d') => {
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.cut_to_clip();
            Transition::Mode(Mode::Normal)
          },
          Mode::Operator('c') => {
            self.send_copy_action_with_text(textarea.selected_text().to_string());
            textarea.cut_to_clip();
            Transition::Mode(Mode::Insert)
          },
          _ => Transition::Nop,
        };
      },
      Mode::Replace => {
        return match input {
          KeyEvent { code: KeyCode::Esc, .. } | KeyEvent { code: KeyCode::Char('c'), modifiers: CONTROL, .. } => {
            Transition::Mode(Mode::Normal)
          },
          input => {
            let c = match input.code {
              KeyCode::Char(c) => Some(c),
              KeyCode::Enter => Some('\n'),
              _ => None,
            };
            if let Some(c) = c {
              textarea.delete_next_char();
              textarea.insert_char(c);
            }
            Transition::Mode(Mode::Normal)
          },
        }
      },
      Mode::Insert => {
        match input {
          KeyEvent { code: KeyCode::Esc, .. } | KeyEvent { code: KeyCode::Char('c'), modifiers: CONTROL, .. } => {
            return Transition::Mode(Mode::Normal);
          },
          KeyEvent { code: KeyCode::Backspace, modifiers: NONE, .. } => {
            textarea.delete_prev_char();
          },
          KeyEvent { code: KeyCode::Backspace, modifiers: ALT, .. } => {
            textarea.delete_prev_word();
          },
          KeyEvent { code: KeyCode::Tab, modifiers: NONE, .. } => {
            textarea.insert_tab();
          },
          KeyEvent { code: KeyCode::Tab, modifiers: SHIFT, .. } => {
            textarea.insert_backtab();
          },
          KeyEvent { code, modifiers: SHIFT, .. } => {
            match code {
              KeyCode::Left => {
                textarea.prev_word_start(textarea.cursor());
              },
              KeyCode::Right => {
                textarea.next_word_end(textarea.cursor());
              },
              KeyCode::Up => {
                textarea.scroll_up(textarea.vertical_page());
              },
              KeyCode::Down => {
                textarea.scroll_down(textarea.vertical_page());
              },
              _ => {},
            }
          },
          input => {
            let c = match input.code {
              KeyCode::Char(c) => Some(c),
              KeyCode::Enter => Some('\n'),
              _ => None,
            };
            if let Some(c) = c {
              textarea.insert_char(c);
            }
          },
        };
        return Transition::Nop;
      },
    };
  }

  fn send_copy_action_with_text(&self, text: String) {
    if let Some(sender) = &self.command_tx {
      sender.send(Action::CopyData(text)).map_or_else(|e| log::error!("{e:?}"), |_| {});
    }
  }
}
