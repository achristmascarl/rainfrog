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
pub mod tui;
pub mod ui;
pub mod utils;
pub mod vim;

use std::{
  io::{self, Write},
  str::FromStr,
};

use clap::Parser;
use cli::Cli;
use color_eyre::eyre::{self, Result};
use database::{BuildConnectionOptions, DatabaseQueries, HasRowsAffected, ValueParser};
use sqlx::{postgres::PgConnectOptions, Connection, Database, Executor, MySql, Pool, Postgres, Sqlite};

use crate::{
  app::App,
  utils::{initialize_logging, initialize_panic_handler, version},
};

async fn run_app<DB>(mut args: Cli) -> Result<()>
where
  DB: Database + BuildConnectionOptions + ValueParser + DatabaseQueries,
  DB::QueryResult: HasRowsAffected,
  for<'c> <DB as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, DB>,
  for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
  let mouse_mode = args.mouse_mode.take();
  let connection_opts = DB::build_connection_opts(args)?;
  let mut app = App::<'_, DB>::new(connection_opts, mouse_mode)?;
  app.run().await?;
  Ok(())
}

async fn tokio_main() -> Result<()> {
  initialize_logging()?;

  initialize_panic_handler()?;

  let args = Cli::parse();
  match args.driver.as_str() {
    "postgres" => run_app::<Postgres>(args).await,
    "mysql" => run_app::<MySql>(args).await,
    "sqlite" => run_app::<Sqlite>(args).await,
    _ => Err(eyre::Report::msg("Please provide a valid a database type")),
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
