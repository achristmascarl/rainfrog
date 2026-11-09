use std::io::{self, Write};

use color_eyre::eyre::{self, WrapErr};
#[cfg(not(target_os = "linux"))]
use keyring::{Entry, Error, Result as KeyringResult};
#[cfg(target_os = "linux")]
use keyring_core::{Entry, Error, Result as KeyringResult, api::CredentialStoreApi};

use crate::Result;

const SERVICE: &str = "rainfrog";

pub struct Password(String);

impl AsRef<str> for Password {
  fn as_ref(&self) -> &str {
    self.0.as_ref()
  }
}

fn storage_unavailable(error: &Error) -> bool {
  matches!(error, Error::NoDefaultStore | Error::NoStorageAccess(_))
    || (cfg!(target_os = "linux") && matches!(error, Error::PlatformFailure(_)))
}

fn read_or_revoke(
  entry: Entry,
  reenter_password: bool,
) -> KeyringResult<(Entry, Option<Password>)> {
  let password = if reenter_password {
    entry.delete_credential().map(|()| None)
  } else {
    entry.get_password().map(|password| Some(Password(password)))
  };
  match password {
    Ok(password) => Ok((entry, password)),
    Err(Error::NoEntry) => Ok((entry, None)),
    Err(error) => Err(error),
  }
}

#[cfg(target_os = "linux")]
fn open_password_entry(
  user: &str,
  reenter_password: bool,
) -> KeyringResult<(Entry, Option<Password>)> {
  let secret_service = zbus_secret_service_keyring_store::Store::new()
    .and_then(|store| store.build(SERVICE, user, None))
    .and_then(|entry| read_or_revoke(entry, reenter_password));
  match secret_service {
    Err(ref error) if storage_unavailable(error) => {},
    result => return result,
  }
  if reenter_password {
    eprintln!("Could not revoke any password saved in Secret Service.");
  }

  // Match keyring v3's Linux key description so existing credentials can be reused.
  let description = format!("keyring-rs:{user}@{SERVICE}");
  let modifiers = std::collections::HashMap::from([("description", description.as_str())]);
  let entry = linux_keyutils_keyring_store::Store::new()?.build(SERVICE, user, Some(&modifiers))?;
  let result = read_or_revoke(entry, reenter_password)?;
  eprintln!("Secret Service unavailable. Using the Linux kernel keyring until reboot or expiry.");
  Ok(result)
}

#[cfg(not(target_os = "linux"))]
fn open_password_entry(
  user: &str,
  reenter_password: bool,
) -> KeyringResult<(Entry, Option<Password>)> {
  read_or_revoke(Entry::new(SERVICE, user)?, reenter_password)
}

fn prompt_for_password(
  entry: Option<&Entry>,
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
    "" | "Y" | "y" => match entry.set_password(password.as_ref()) {
      Ok(()) => println!("Password saved in keyring"),
      Err(error) if storage_unavailable(&error) => {
        eprintln!("Credential store unavailable. Using the password for this connection only.");
      },
      Err(error) => return Err(error).wrap_err("Failed to save password in keyring"),
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
  let user = format!("{connection_name}-{username}");
  let entry = match open_password_entry(&user, reenter_password) {
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
