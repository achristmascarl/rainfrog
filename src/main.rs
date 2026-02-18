#![allow(unused_variables)]
#![allow(async_fn_in_trait)]
#![warn(unused_extern_crates)]

pub mod action;
pub mod app;
pub mod cli;
pub mod components;
pub mod config;
pub mod database;
pub mod focus;
pub mod keyring;
pub mod popups;
pub mod tui;
pub mod ui;
pub mod utils;
pub mod vim;

use std::{
  env,
  io::{self, Write},
};

use clap::Parser;
use cli::{Cli, Driver, extract_driver_from_url, extract_port_and_database_from_url, prompt_for_database_selection};
use color_eyre::eyre::Result;
use config::{Config, ConnectionString};
use dotenvy::dotenv;
use keyring::get_password;

use crate::{
  app::App,
  utils::{initialize_logging, initialize_panic_handler},
};

async fn run_app(mut args: Cli, config: Config, driver: Driver) -> Result<()> {
  let mouse_mode = args.mouse_mode.take();
  let mut app = App::new(mouse_mode, config)?;
  app.run(driver, args).await?;
  Ok(())
}

fn resolve_driver(args: &mut Cli, config: &Config) -> Result<Driver> {
  let url = args.connection_url.clone().or_else(|| {
    env::var("DATABASE_URL").map_or(None, |url| {
      if url.is_empty() {
        None
      } else {
        println!("Using DATABASE_URL from environment variable");
        Some(url)
      }
    })
  });
  let has_cli_input = args.driver.is_some()
    || args.user.is_some()
    || args.password.is_some()
    || args.host.is_some()
    || args.port.is_some()
    || args.database.is_some();

  args.connection_name = None;

  let (driver, url) = match (url, has_cli_input) {
    (Some(u), _) => {
      if let Some(driver) = args.driver.take() { Ok(driver) } else { extract_driver_from_url(&u) }.map(|d| (d, Some(u)))
    },
    (None, true) => {
      if let Some(driver) = args.driver.take() {
        Ok((driver, None))
      } else {
        Ok((prompt_for_driver()?, None))
      }
    },
    (None, false) => Ok(match prompt_for_database_selection(config)? {
      Some((conn, name)) => {
        args.connection_name = Some(name.clone());
        let url = match conn.connection {
          ConnectionString::Raw { connection_string } => Ok(connection_string),
          ConnectionString::Structured { details } => {
            let password = get_password(&name, &details.username)?;
            details.connection_string(conn.driver, password)
          },
        }?;

        (conn.driver, Some(url))
      },
      None => (prompt_for_driver()?, None),
    }),
  }?;

  args.connection_url = url;
  if args.connection_name.is_none() {
    let extracted = args.connection_url.as_deref().and_then(extract_port_and_database_from_url);
    let extracted_port = extracted.as_ref().map(|(port, _)| *port);
    let extracted_database = extracted.as_ref().map(|(_, database)| database.clone());
    let port = args.port.or(extracted_port);
    let database = args.database.clone().or(extracted_database);
    if let (Some(port), Some(database)) = (port, database) {
      args.connection_name = Some(format!("{port}/{database}"));
    }
  }

  Ok(driver)
}

async fn tokio_main() -> Result<()> {
  initialize_logging()?;

  initialize_panic_handler()?;

  let mut args = Cli::parse();
  dotenv().ok();
  let config = Config::new()?;
  let driver = resolve_driver(&mut args, &config)?;

  run_app(args, config, driver).await
}

#[tokio::main]
async fn main() -> Result<()> {
  match tokio_main().await {
    Err(e) => {
      eprintln!("{} error: Something went wrong", env!("CARGO_PKG_NAME"));
      Err(e)
    },
    _ => Ok(()),
  }
}

pub fn prompt_for_driver() -> Result<Driver> {
  let mut driver = String::new();
  print!("Database driver (postgres, mysql, sqlite, oracle, duckdb): ");
  io::stdout().flush()?;
  io::stdin().read_line(&mut driver)?;
  driver.trim().to_lowercase().parse()
}
