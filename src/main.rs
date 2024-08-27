#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

pub mod action;
pub mod app;
pub mod cli;
pub mod components;
pub mod config;
pub mod database;
pub mod focus;
pub mod tui;
pub mod ui;
pub mod utils;
pub mod vim;

use clap::Parser;
use cli::Cli;
use color_eyre::eyre::Result;

#[cfg(not(feature = "ish"))]
use crate::utils::initialize_logging;
#[cfg(not(feature = "ish"))]
use crate::utils::initialize_panic_handler;
use crate::{app::App, utils::version};

async fn tokio_main() -> Result<()> {
  #[cfg(not(feature = "ish"))]
  {
    initialize_logging()?;
    initialize_panic_handler()?;
  }

  let args = Cli::parse();
  let mut app = App::new(args.connection_url, args.tick_rate, args.frame_rate)?;
  app.run().await?;

  Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
  if let Err(e) = tokio_main().await {
    eprintln!("{} error: Something went wrong", env!("CARGO_PKG_NAME"));
    Err(e)
  } else {
    Ok(())
  }
}
