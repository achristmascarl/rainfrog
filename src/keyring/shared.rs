use std::{
  collections::BTreeMap,
  fmt,
  fs::{self, File, OpenOptions},
  io,
  os::unix::fs::OpenOptionsExt,
  path::{Path, PathBuf},
};

use color_eyre::eyre::{WrapErr, eyre};
use serde::{Deserialize, Serialize};

use super::{Entry, KeyringResult, Password, allow_missing, legacy_account};
use crate::{Result, utils::project_directory};

// Legacy account names always contain a hyphen.
const SHARED_ACCOUNT: &str = "secrets";

#[derive(Debug)]
pub(super) struct LockUnavailable;

impl fmt::Display for LockUnavailable {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str("Could not lock shared credentials")
  }
}

impl std::error::Error for LockUnavailable {}

#[derive(Default, Deserialize, Serialize)]
struct Secrets {
  #[serde(default)]
  passwords: BTreeMap<String, ConnectionPasswords>,
  #[serde(flatten)]
  other: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ConnectionPasswords {
  ByUsername(BTreeMap<String, String>),
  // Earlier shared entries lack a username, so preserve them until a scoped password is saved.
  Unscoped(String),
}

impl Secrets {
  fn password(&self, connection_name: &str, username: &str) -> Option<&String> {
    match self.passwords.get(connection_name)? {
      ConnectionPasswords::ByUsername(passwords) => passwords.get(username),
      ConnectionPasswords::Unscoped(_) => None,
    }
  }

  fn insert_password(&mut self, connection_name: &str, username: &str, password: &str) {
    let username = username.to_owned();
    let password = password.to_owned();
    match self.passwords.get_mut(connection_name) {
      Some(ConnectionPasswords::ByUsername(passwords)) => {
        passwords.insert(username, password);
      },
      _ => {
        self.passwords.insert(
          connection_name.to_owned(),
          ConnectionPasswords::ByUsername(BTreeMap::from([(username, password)])),
        );
      },
    }
  }

  fn remove_password(&mut self, connection_name: &str, username: &str) -> bool {
    let Some(ConnectionPasswords::ByUsername(passwords)) = self.passwords.get_mut(connection_name)
    else {
      return false;
    };
    let removed = passwords.remove(username).is_some();
    if passwords.is_empty() {
      self.passwords.remove(connection_name);
    }
    removed
  }
}

pub(super) struct PasswordEntry {
  shared: Entry,
  legacy: Entry,
  connection_name: String,
  username: String,
  lock_path: PathBuf,
}

impl PasswordEntry {
  pub(super) fn new(
    connection_name: &str,
    username: &str,
    build: impl Fn(&str) -> KeyringResult<Entry>,
  ) -> Result<Self> {
    // The keyring is shared across Rainfrog configs, including when RAINFROG_DATA is overridden.
    let lock_path =
      project_directory().ok_or(LockUnavailable)?.data_local_dir().join("secrets.lock");
    Ok(Self {
      shared: build(SHARED_ACCOUNT)?,
      legacy: build(&legacy_account(connection_name, username))?,
      connection_name: connection_name.to_owned(),
      username: username.to_owned(),
      lock_path,
    })
  }

  pub(super) fn read_or_revoke(self, reenter_password: bool) -> Result<(Self, Option<Password>)> {
    let _lock = lock_secrets(&self.lock_path).wrap_err(LockUnavailable)?;
    let mut secrets = self.load()?;
    if reenter_password {
      // Clear the old entry first so a later lookup cannot migrate a revoked password back.
      allow_missing(self.legacy.delete_credential())?;
      if secrets.remove_password(&self.connection_name, &self.username) {
        self.save(&secrets)?;
      }
      return Ok((self, None));
    }

    if let Some(password) = secrets.password(&self.connection_name, &self.username) {
      return Ok((self, Some(Password(password.clone()))));
    }
    let password = allow_missing(self.legacy.get_password())?;
    if let Some(ref password) = password {
      secrets.insert_password(&self.connection_name, &self.username, password);
      // Verify the stored copy before removing the only known-good credential.
      let migrated = self.save(&secrets).and_then(|()| self.load()).is_ok_and(|stored| {
        stored.password(&self.connection_name, &self.username) == Some(password)
      });
      if !migrated {
        eprintln!("Could not migrate saved password. Using the existing keyring entry.");
      } else if allow_missing(self.legacy.delete_credential()).is_err() {
        eprintln!("Password migrated, but the old keyring entry could not be removed.");
      }
    }
    Ok((self, password.map(Password)))
  }

  pub(super) fn set_password(&self, password: &str) -> Result<()> {
    let _lock = lock_secrets(&self.lock_path).wrap_err(LockUnavailable)?;
    let mut secrets = self.load()?;
    secrets.insert_password(&self.connection_name, &self.username, password);
    self.save(&secrets)
  }

  fn load(&self) -> Result<Secrets> {
    let Some(json) = allow_missing(self.shared.get_secret())? else {
      return Ok(Secrets::default());
    };
    // A deserialization error can quote the unexpected value, which may itself be a secret.
    serde_json::from_slice(&json).map_err(|_| eyre!("Invalid shared credential data"))
  }

  fn save(&self, secrets: &Secrets) -> Result<()> {
    let json =
      serde_json::to_vec(secrets).map_err(|_| eyre!("Could not encode shared credentials"))?;
    Ok(self.shared.set_secret(&json)?)
  }
}

fn lock_secrets(path: &Path) -> io::Result<File> {
  fs::create_dir_all(path.parent().expect("credential lock has a parent directory"))?;
  let file = OpenOptions::new()
    .write(true)
    .create(true)
    .truncate(false)
    .mode(0o600)
    .custom_flags(libc::O_NOFOLLOW)
    .open(path)?;
  loop {
    match file.lock() {
      Ok(()) => return Ok(file),
      Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
      Err(error) => return Err(error),
    }
  }
}

#[cfg(test)]
mod tests {
  use keyring_core::{Error, api::CredentialStoreApi, mock::Store};
  use serde_json::{Value, json};

  use super::*;
  use crate::keyring::SERVICE;

  fn mock_entry(store: &Store, account: &str) -> Entry {
    let entry = store.build(SERVICE, account, None).unwrap();
    #[cfg(target_os = "macos")]
    let entry = Entry { inner: entry };
    entry
  }

  fn connection(store: &Store, lock_path: &Path, name: &str, username: &str) -> PasswordEntry {
    PasswordEntry {
      shared: mock_entry(store, SHARED_ACCOUNT),
      legacy: mock_entry(store, &legacy_account(name, username)),
      connection_name: name.to_owned(),
      username: username.to_owned(),
      lock_path: lock_path.to_owned(),
    }
  }

  #[test]
  fn saved_passwords_migrate_on_use_and_can_be_reentered_per_username() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("secrets.lock");
    let store = Store::new().unwrap();
    let shared = mock_entry(&store, SHARED_ACCOUNT);
    let first_legacy = mock_entry(&store, "local-alice");
    let second_legacy = mock_entry(&store, "local-bob");
    first_legacy.set_password("alice-pass").unwrap();
    second_legacy.set_password("bob-pass").unwrap();
    shared
      .set_secret(
        br#"{"passwords":{"local":"unknown-owner","other":"unused"},"tokens":{"api":"test-token"}}"#,
      )
      .unwrap();

    let (_, password) =
      connection(&store, &lock_path, "local", "charlie").read_or_revoke(false).unwrap();
    assert!(password.is_none());
    let (_, password) =
      connection(&store, &lock_path, "local", "alice").read_or_revoke(false).unwrap();
    assert_eq!(password.unwrap().as_ref(), "alice-pass");
    assert!(matches!(first_legacy.get_password(), Err(Error::NoEntry)));
    assert_eq!(second_legacy.get_password().unwrap(), "bob-pass");

    // Two open connections update the same bundle at different points in their workflows.
    let (new, password) =
      connection(&store, &lock_path, "local", "charlie").read_or_revoke(false).unwrap();
    assert!(password.is_none());
    let (_, password) =
      connection(&store, &lock_path, "local", "bob").read_or_revoke(false).unwrap();
    assert_eq!(password.unwrap().as_ref(), "bob-pass");
    assert!(matches!(second_legacy.get_password(), Err(Error::NoEntry)));
    new.set_password("new-pass").unwrap();

    // A leftover legacy entry must not restore a revoked password.
    first_legacy.set_password("old-pass").unwrap();
    let (local, password) =
      connection(&store, &lock_path, "local", "alice").read_or_revoke(true).unwrap();
    assert!(password.is_none());
    assert!(matches!(first_legacy.get_password(), Err(Error::NoEntry)));
    let (_, password) =
      connection(&store, &lock_path, "local", "alice").read_or_revoke(false).unwrap();
    assert!(password.is_none());
    let (_, password) =
      connection(&store, &lock_path, "local", "bob").read_or_revoke(false).unwrap();
    assert_eq!(password.unwrap().as_ref(), "bob-pass");
    local.set_password("replacement").unwrap();

    let contents: Value = serde_json::from_slice(&shared.get_secret().unwrap()).unwrap();
    assert_eq!(
      contents,
      json!({
        "passwords": {
          "local": {"alice": "replacement", "bob": "bob-pass", "charlie": "new-pass"},
          "other": "unused"
        },
        "tokens": {"api": "test-token"}
      })
    );
  }

  #[test]
  fn malformed_bundle_leaves_saved_credentials_intact() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("secrets.lock");
    let store = Store::new().unwrap();
    let shared = mock_entry(&store, SHARED_ACCOUNT);
    let legacy = mock_entry(&store, "local-alice");
    let invalid = br#"{"passwords":"sensitive-data"}"#;
    shared.set_secret(invalid).unwrap();
    legacy.set_password("legacy-pass").unwrap();

    for reenter in [false, true] {
      let error =
        connection(&store, &lock_path, "local", "alice").read_or_revoke(reenter).err().unwrap();
      assert_eq!(error.to_string(), "Invalid shared credential data");
      assert_eq!(shared.get_secret().unwrap(), invalid);
      assert_eq!(legacy.get_password().unwrap(), "legacy-pass");
    }
  }
}
