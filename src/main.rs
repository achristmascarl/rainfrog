#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
// for some reason, clippy thinks the tokio::main fn has a needless return...
#![allow(clippy::needless_return)]

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
  str::FromStr,
};

use clap::Parser;
use cli::{extract_driver_from_url, prompt_for_database_selection, Cli, Driver};
use color_eyre::eyre::{self, Result};
use config::{Config, ConnectionString};
use database::{BuildConnectionOptions, DatabaseQueries, HasRowsAffected, ValueParser};
use dotenvy::dotenv;
use keyring::get_password;
use sqlx::{postgres::PgConnectOptions, Connection, Database, Executor, MySql, Pool, Postgres, Sqlite};

use crate::{
  app::App,
  utils::{initialize_logging, initialize_panic_handler, version},
};

async fn run_app<DB>(mut args: Cli, config: Config) -> Result<()>
where
  DB: Database + BuildConnectionOptions + ValueParser + DatabaseQueries,
  DB::QueryResult: HasRowsAffected,
  for<'c> <DB as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, DB>,
  for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
  let mouse_mode = args.mouse_mode.take();
  let connection_opts = DB::build_connection_opts(args)?;
  let mut app = App::<'_, DB>::new(connection_opts, mouse_mode, config)?;
  app.run().await?;
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

  let (driver, url) = match url {
    Some(u) => {
      if let Some(driver) = args.driver.take() { Ok(driver) } else { extract_driver_from_url(&u) }.map(|d| (d, Some(u)))
    },
    None => {
      Ok(match prompt_for_database_selection(config)? {
        Some((conn, name)) => {
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
      })
    },
  }?;

  args.connection_url = url;

  Ok(driver)
}

async fn tokio_main() -> Result<()> {
  initialize_logging()?;

  initialize_panic_handler()?;

  let mut args = Cli::parse();
  dotenv().ok();

  let config = Config::new()?;

  let driver = resolve_driver(&mut args, &config)?;

  match driver {
    Driver::Postgres => run_app::<Postgres>(args, config).await,
    Driver::Mysql => run_app::<MySql>(args, config).await,
    Driver::Sqlite => run_app::<Sqlite>(args, config).await,
  }
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

pub fn prompt_for_driver() -> Result<Driver> {
  let mut driver = String::new();
  print!("Database driver (postgres, mysql, sqlite): ");
  io::stdout().flush()?;
  io::stdin().read_line(&mut driver)?;
  driver.trim().to_lowercase().parse()
}
