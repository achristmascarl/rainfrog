use std::io::{self, Write};

use color_eyre::eyre::{self, WrapErr};
use keyring::Entry;

use crate::Result;

pub struct Password(String);

impl AsRef<str> for Password {
  fn as_ref(&self) -> &str {
    self.0.as_ref()
  }
}

fn revoke_password(entry: &Entry) -> Result<()> {
  match entry.delete_credential() {
    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
    Err(e) => Err(e).wrap_err("Failed to revoke password from keyring"),
  }
}

fn prompt_for_password(entry: &Entry, connection_name: &str, username: &str) -> Result<Password> {
  println!("{username}@{connection_name}");
  let password = rpassword::prompt_password("Password: ")?;

  print!("Save password (Y/n): ");
  let mut save = String::new();
  io::stdout().flush()?;
  io::stdin().read_line(&mut save)?;
  match save.trim() {
    "" | "Y" | "y" => {
      entry.set_password(&password)?;
      println!("Password saved in keyring");
      Ok(())
    },
    "n" | "N" => {
      println!("Password not saved in keyring");
      Ok(())
    },
    _ => Err(eyre::Report::msg("Unrecognized save option")),
  }?;

  Ok(Password(password))
}

pub fn get_password(
  connection_name: &str,
  username: &str,
  reenter_password: bool,
) -> Result<Password> {
  let entry = Entry::new("rainfrog", &format!("{connection_name}-{username}"))?;

  if reenter_password {
    revoke_password(&entry)?;
    return prompt_for_password(&entry, connection_name, username);
  }

  match entry.get_password() {
    Ok(password) => {
      println!("Password found in keyring");
      Ok(Password(password))
    },
    Err(e) => match e {
      keyring::Error::NoEntry => prompt_for_password(&entry, connection_name, username),
      _ => Err(eyre::Report::msg("Failed to extract password from secret: {e:?}")),
    },
  }
}
