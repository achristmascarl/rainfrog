use color_eyre::eyre::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
  action::Action,
  app::AppState,
  config::Config,
  tui::{Event, Frame},
};

pub mod data;
pub mod editor;
pub mod history;
pub mod menu;
pub mod scroll_table;
pub trait Component<DB: sqlx::Database> {
  /// Register an action handler that can send actions for processing if necessary.
  ///
  /// # Arguments
  ///
  /// * `tx` - An unbounded sender that can send actions.
  ///
  /// # Returns
  ///
  /// * `Result<()>` - An Ok result or an error.
  #[allow(unused_variables)]
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    Ok(())
  }
  /// Register a configuration handler that provides configuration settings if necessary.
  ///
  /// # Arguments
  ///
  /// * `config` - Configuration settings.
  ///
  /// # Returns
  ///
  /// * `Result<()>` - An Ok result or an error.
  #[allow(unused_variables)]
  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    Ok(())
  }
  /// Initialize the component with a specified area if necessary.
  ///
  /// # Arguments
  ///
  /// * `area` - Rectangular area to initialize the component within.
  ///
  /// # Returns
  ///
  /// * `Result<()>` - An Ok result or an error.
  fn init(&mut self, area: Rect) -> Result<()> {
    Ok(())
  }
  /// Handle incoming events and produce actions if necessary.
  ///
  /// # Arguments
  ///
  /// * `event` - An optional event to be processed.
  ///
  /// # Returns
  ///
  /// * `Result<Option<Action>>` - An action to be processed or none.
  fn handle_events(
    &mut self,
    event: Option<Event>,
    last_tick_key_events: Vec<KeyEvent>,
    app_state: &AppState<'_, DB>,
  ) -> Result<Option<Action>> {
    let r = match event {
      Some(Event::Key(key_event)) => self.handle_key_events(key_event, app_state)?,
      Some(Event::Mouse(mouse_event)) => self.handle_mouse_events(mouse_event, app_state)?,
      _ => None,
    };
    Ok(r)
  }
  /// Handle key events and produce actions if necessary.
  ///
  /// # Arguments
  ///
  /// * `key` - A key event to be processed.
  ///
  /// # Returns
  ///
  /// * `Result<Option<Action>>` - An action to be processed or none.
  #[allow(unused_variables)]
  fn handle_key_events(&mut self, key: KeyEvent, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    Ok(None)
  }
  /// Handle mouse events and produce actions if necessary.
  ///
  /// # Arguments
  ///
  /// * `mouse` - A mouse event to be processed.
  ///
  /// # Returns
  ///
  /// * `Result<Option<Action>>` - An action to be processed or none.
  #[allow(unused_variables)]
  fn handle_mouse_events(&mut self, mouse: MouseEvent, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    Ok(None)
  }
  /// Update the state of the component based on a received action. (REQUIRED)
  ///
  /// # Arguments
  ///
  /// * `action` - An action that may modify the state of the component.
  ///
  /// # Returns
  ///
  /// * `Result<Option<Action>>` - An action to be processed or none.
  #[allow(unused_variables)]
  fn update(&mut self, action: Action, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    Ok(None)
  }
  /// Render the component on the screen. (REQUIRED)
  ///
  /// # Arguments
  ///
  /// * `f` - A frame used for rendering.
  /// * `area` - The area in which the component should be drawn.
  ///
  /// # Returns
  ///
  /// * `Result<()>` - An Ok result or an error.
  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState<'_, DB>) -> Result<()>;
}
