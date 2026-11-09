use std::io::{self, Write};

use color_eyre::eyre::{self, WrapErr};
#[cfg(not(target_os = "linux"))]
use keyring::{Entry, Error, Result as KeyringResult};
#[cfg(target_os = "linux")]
use keyring_core::{Entry, Error, Result as KeyringResult, api::CredentialStoreApi};

use crate::Result;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shared;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use shared::PasswordEntry;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
type PasswordEntry = Entry;

const SERVICE: &str = "rainfrog";

pub struct Password(String);

impl AsRef<str> for Password {
  fn as_ref(&self) -> &str {
    self.0.as_ref()
  }
}

fn storage_unavailable(error: &eyre::Report) -> bool {
  let error = error.downcast_ref::<Error>();
  matches!(error, Some(Error::NoDefaultStore | Error::NoStorageAccess(_)))
    || (cfg!(target_os = "linux") && matches!(error, Some(Error::PlatformFailure(_))))
}

fn allow_missing<T>(result: KeyringResult<T>) -> KeyringResult<Option<T>> {
  match result {
    Ok(value) => Ok(Some(value)),
    Err(Error::NoEntry) => Ok(None),
    Err(error) => Err(error),
  }
}

fn legacy_account(connection_name: &str, username: &str) -> String {
  format!("{connection_name}-{username}")
}

#[cfg(target_os = "linux")]
fn open_password_entry(
  connection_name: &str,
  username: &str,
  reenter_password: bool,
) -> Result<(PasswordEntry, Option<Password>)> {
  let secret_service = (|| {
    let store = zbus_secret_service_keyring_store::Store::new()?;
    PasswordEntry::new(connection_name, username, |account| store.build(SERVICE, account, None))?
      .read_or_revoke(reenter_password)
  })();
  match secret_service {
    Err(ref error) if storage_unavailable(error) => {},
    result => return result,
  }
  if reenter_password {
    eprintln!("Could not revoke any password saved in Secret Service.");
  }

  let store = linux_keyutils_keyring_store::Store::new()?;
  let entry = PasswordEntry::new(connection_name, username, |account| {
    // Match keyring v3's Linux key description so existing credentials can be reused.
    let description = format!("keyring-rs:{account}@{SERVICE}");
    let modifiers = std::collections::HashMap::from([("description", description.as_str())]);
    store.build(SERVICE, account, Some(&modifiers))
  })?;
  let result = entry.read_or_revoke(reenter_password)?;
  eprintln!("Secret Service unavailable. Using the Linux kernel keyring until reboot or expiry.");
  Ok(result)
}

#[cfg(target_os = "macos")]
fn open_password_entry(
  connection_name: &str,
  username: &str,
  reenter_password: bool,
) -> Result<(PasswordEntry, Option<Password>)> {
  PasswordEntry::new(connection_name, username, |account| Entry::new(SERVICE, account))?
    .read_or_revoke(reenter_password)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn open_password_entry(
  connection_name: &str,
  username: &str,
  reenter_password: bool,
) -> Result<(PasswordEntry, Option<Password>)> {
  let entry = Entry::new(SERVICE, &legacy_account(connection_name, username))?;
  let password = if reenter_password {
    allow_missing(entry.delete_credential())?;
    None
  } else {
    allow_missing(entry.get_password())?.map(Password)
  };
  Ok((entry, password))
}

fn prompt_for_password(
  entry: Option<&PasswordEntry>,
  connection_name: &str,
  username: &str,
) -> Result<Password> {
  println!("{username}@{connection_name}");
  let password = Password(rpassword::prompt_password("Password: ")?);
  let Some(entry) = entry else {
    return Ok(password);
  };

  print!("Save password (Y/n): ");
  let mut save = String::new();
  io::stdout().flush()?;
  io::stdin().read_line(&mut save)?;
  match save.trim() {
    "" | "Y" | "y" => {
      match entry.set_password(password.as_ref()).wrap_err("Failed to save password in keyring") {
        Ok(()) => println!("Password saved in keyring"),
        Err(error) if storage_unavailable(&error) => {
          eprintln!("Credential store unavailable. Using the password for this connection only.");
        },
        Err(error) => return Err(error),
      }
    },
    "n" | "N" => println!("Password not saved in keyring"),
    _ => return Err(eyre::Report::msg("Unrecognized save option")),
  }

  Ok(password)
}

pub fn get_password(
  connection_name: &str,
  username: &str,
  reenter_password: bool,
) -> Result<Password> {
  let entry = match open_password_entry(connection_name, username, reenter_password) {
    Ok((_, Some(password))) => {
      println!("Password found in keyring");
      return Ok(password);
    },
    Ok((entry, None)) => Some(entry),
    Err(error) if storage_unavailable(&error) => {
      if reenter_password {
        eprintln!("Could not revoke any password in the unavailable credential store.");
      }
      eprintln!("Credential store unavailable. Using the password for this connection only.");
      None
    },
    Err(error) if reenter_password => {
      return Err(error).wrap_err("Failed to revoke password from keyring");
    },
    Err(error) => return Err(error).wrap_err("Failed to read password from keyring"),
  };
  prompt_for_password(entry.as_ref(), connection_name, username)
}
