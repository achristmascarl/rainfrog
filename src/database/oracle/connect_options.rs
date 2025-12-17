use std::{
  io::{self, Write as _},
  str::FromStr,
};

use color_eyre::eyre::Result;

pub struct OracleConnectOptions {
  user: Option<String>,
  password: Option<String>,
  host: String,
  port: Option<u16>,
  database: String,
}
impl OracleConnectOptions {
  fn new() -> Self {
    Self { user: None, password: None, host: "localhost".to_string(), port: Some(1521), database: "XE".to_string() }
  }

  fn get_connection_string(&self) -> String {
    let port = self.port.unwrap_or(1521u16);
    format!("//{}:{}/{}", self.host, port, self.database)
  }
  pub fn get_connection_options(&self) -> std::result::Result<(String, String, String), String> {
    let user = self.user.clone().ok_or("User is required for Oracle connection".to_string())?;
    let password = self.password.clone().ok_or("Password is required for Oracle connection".to_string())?;
    let connection_string = self.get_connection_string();
    Ok((user, password, connection_string))
  }

  pub fn build_connection_opts(args: crate::cli::Cli) -> Result<OracleConnectOptions> {
    match args.connection_url {
      Some(url) => {
        let mut opts = OracleConnectOptions::from_str(&url).map_err(|e| color_eyre::eyre::eyre!(e))?;

        // Username
        if opts.user.is_none() {
          if let Some(user) = args.user {
            opts.user = Some(user);
          } else {
            let mut user = String::new();
            print!("username: ");
            io::stdout().flush()?;
            io::stdin().read_line(&mut user)?;
            let user = user.trim();
            if !user.is_empty() {
              opts.user = Some(user.to_string());
            }
          }
        }

        // Password
        if opts.password.is_none() {
          if let Some(password) = args.password {
            opts.password = Some(password);
          } else {
            let password = rpassword::prompt_password(format!(
              "password for user {}: ",
              opts.user.clone().unwrap_or("".to_string())
            ))
            .unwrap();
            let password = password.trim();
            if !password.is_empty() {
              opts.password = Some(password.to_string());
            }
          }
        }

        Ok(opts)
      },
      None => {
        let mut opts = OracleConnectOptions::new();

        // Username
        if let Some(user) = args.user {
          opts.user = Some(user);
        } else {
          let mut user = String::new();
          print!("username: ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut user)?;
          let user = user.trim();
          if !user.is_empty() {
            opts.user = Some(user.to_string());
          }
        }

        // Password
        if let Some(password) = args.password {
          opts.password = Some(password);
        } else {
          let password =
            rpassword::prompt_password(format!("password for user {}: ", opts.user.clone().unwrap_or("".to_string())))
              .unwrap();
          let password = password.trim();
          if !password.is_empty() {
            opts.password = Some(password.to_string());
          }
        }

        // Host
        if let Some(host) = args.host {
          opts.host = host;
        } else {
          let mut host = String::new();
          print!("host (ex. localhost): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut host)?;
          let host = host.trim();
          if !host.is_empty() {
            opts.host = host.to_string();
          }
        }

        // Port
        if let Some(port) = args.port {
          opts.port = Some(port);
        } else {
          let mut port = String::new();
          print!("port (ex. 1521): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut port)?;
          let port = port.trim();
          if !port.is_empty() {
            opts.port = Some(port.parse()?);
          }
        }

        // Database
        if let Some(database) = args.database {
          opts.database = database;
        } else {
          let mut database = String::new();
          print!("database (ex. mydb): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim();
          if !database.is_empty() {
            opts.database = database.to_string();
          }
        }

        Ok(opts)
      },
    }
  }
}

impl FromStr for OracleConnectOptions {
  type Err = String;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    let s = s.trim().trim_start_matches("jdbc:oracle:thin:").trim_start_matches("jdbc:");
    let (is_easy_connect, (auth_part, host_part)) = if s.contains("@//") {
      (true, s.split_once("@//").ok_or("Invalid Oracle Easy Connect connection string format".to_string())?)
    } else if s.contains("@") {
      (false, s.split_once('@').ok_or("Invalid Oracle SID connection string format".to_string())?)
    } else {
      return Err("Invalid Oracle connection string format".to_string());
    };

    let (user, password) = if auth_part.contains('/') {
      let (user, password) = auth_part.split_once('/').ok_or("Invalid Oracle connection string format".to_string())?;
      (Some(user.to_string()), Some(password.to_string()))
    } else if !auth_part.is_empty() {
      (Some(auth_part.to_string()), None)
    } else {
      (None, None)
    };

    let (host, port, database) = if is_easy_connect {
      let (host_port, database) =
        host_part.split_once('/').ok_or("Invalid Oracle Easy Connect connection string format".to_string())?;

      let (host, port) = if host_port.contains(':') {
        let (host, port) =
          host_port.split_once(':').ok_or("Invalid Oracle Easy Connect connection string format".to_string())?;
        let port = port.parse().map_err(|_| "Invalid port")?;
        (host.to_string(), Some(port))
      } else {
        (host_port.to_string(), None)
      };

      (host, port, database.to_string())
    } else {
      let parts = host_part.split(':').collect::<Vec<_>>();
      if parts.len() == 2 {
        (parts[0].to_string(), None, parts[1].to_string())
      } else if parts.len() == 3 {
        let port = parts[1].parse().map_err(|_| "Invalid port")?;
        (parts[0].to_string(), Some(port), parts[2].to_string())
      } else {
        return Err("Invalid Oracle SID connection string format".to_string());
      }
    };

    Ok(OracleConnectOptions { user, password, host, port, database })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn test_oracle_connect_options_from_easy_connect_string() {
    let opts = OracleConnectOptions::from_str("jdbc:oracle:thin:user/password@//localhost:1521/XE").unwrap();
    assert_eq!(opts.user, Some("user".to_string()));
    assert_eq!(opts.password, Some("password".to_string()));
    assert_eq!(opts.host, "localhost");
    assert_eq!(opts.port, Some(1521));
    assert_eq!(opts.database, "XE");
  }

  #[test]
  fn test_oracle_connect_options_from_sid_string() {
    let opts = OracleConnectOptions::from_str("jdbc:oracle:thin:user/password@localhost:1521:XE").unwrap();
    assert_eq!(opts.user, Some("user".to_string()));
    assert_eq!(opts.password, Some("password".to_string()));
    assert_eq!(opts.host, "localhost");
    assert_eq!(opts.port, Some(1521));
    assert_eq!(opts.database, "XE");
  }

  #[test]
  fn test_oracle_connect_options_get_connection_string() {
    let opts = OracleConnectOptions::new();
    assert_eq!(opts.get_connection_string(), "//localhost:1521/XE");
  }
}
