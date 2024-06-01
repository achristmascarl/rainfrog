use std::path::PathBuf;

use clap::Parser;

use crate::utils::version;

#[derive(Parser, Debug)]
#[command(author, version = version(), about)]
pub struct Cli {
  #[arg(short, long, value_name = "FLOAT", help = "Tick rate, i.e. number of ticks per second")]
  pub tick_rate: Option<f64>,

  #[arg(short, long, value_name = "FLOAT", help = "Frame rate, i.e. number of frames per second")]
  pub frame_rate: Option<f64>,

  #[arg(
    short = 'u',
    long = "url",
    value_name = "URL",
    help = "Connection URL for the database, e.g. postgres://username::password@localhost:5432/dbname"
  )]
  pub connection_url: String,
}
