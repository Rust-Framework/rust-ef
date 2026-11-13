//! Connection-string parsing, building and validation (EF-15).
//!
//! A code-first installer turns form fields into a database URL. Doing that by
//! hand tends to forget percent-encoding a password, accept port `0`, or emit a
//! TLS mode the driver does not understand. [`ConnectionString`] makes the
//! conversion explicit and typed:
//!
//! ```ignore
//! use rust_ef::connection_string::{ConnectionProtocol, ConnectionString};
//!
//! let cs = ConnectionString {
//!     protocol: ConnectionProtocol::Postgres,
//!     username: Some("travel".into()),
//!     password: Some("p@ss word".into()),
//!     host: Some("127.0.0.1".into()),
//!     port: Some(5432),
//!     database: Some("travel".into()),
//!     options: vec![("sslmode".into(), "require".into())],
//! };
//! assert_eq!(
//!     cs.build().unwrap(),
//!     "postgres://travel:p%40ss%20word@127.0.0.1:5432/travel?sslmode=require"
//! );
//! assert_eq!(ConnectionString::parse(&cs.build().unwrap()).unwrap(), cs);
//! ```
//!
//! SQLite targets are file paths, not URLs, so they are represented by
//! [`ProviderKind::Sqlite`] and are rejected by [`ConnectionString`].

use crate::error::EFError;
use crate::migration::MigrationDialect;
use std::fmt;

/// PostgreSQL TLS modes accepted by libpq / sqlx.
pub const POSTGRES_SSL_MODES: &[&str] = &[
    "disable",
    "allow",
    "prefer",
    "require",
    "verify-ca",
    "verify-full",
];

/// MySQL TLS modes accepted by sqlx's MySQL connector.
pub const MYSQL_SSL_MODES: &[&str] = &[
    "disabled",
    "preferred",
    "required",
    "verify_ca",
    "verify_identity",
];

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Typed errors returned by [`ConnectionString`] and [`ProviderKind`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionStringError {
    /// The string is not a URL (`<protocol>://...`).
    #[error("invalid connection string: {0}")]
    Malformed(String),
    /// The URL protocol is not one of the supported network databases.
    #[error(
        "unsupported database protocol '{0}'; supported: postgres, postgresql, mysql, mariadb"
    )]
    UnsupportedProtocol(String),
    /// The configured provider name is unknown.
    #[error(
        "unsupported database provider '{0}'; supported: sqlite, mysql, mariadb, postgres, postgresql"
    )]
    UnsupportedProvider(String),
    /// The port is not an integer in `1-65535`.
    #[error("invalid port '{0}'; expected an integer in 1-65535")]
    InvalidPort(String),
    /// The TLS mode is not in the protocol's whitelist.
    #[error("invalid {0} '{1}'; supported: {2}")]
    InvalidSslMode(&'static str, String, String),
    /// The host contains unsupported characters.
    #[error("invalid host '{0}'; expected a hostname, IPv4 or bracketed IPv6 address")]
    InvalidHost(String),
    /// The database name contains unsupported characters.
    #[error("invalid database name '{0}'; expected letters, digits, '_', '-' or '.'")]
    InvalidDatabase(String),
    /// A password was given without a username.
    #[error("connection string has a password but no username")]
    PasswordWithoutUsername,
}

impl From<ConnectionStringError> for EFError {
    fn from(err: ConnectionStringError) -> Self {
        EFError::configuration(err.to_string()).with_source(Box::new(err))
    }
}

// ---------------------------------------------------------------------------
// Protocol / provider kinds
// ---------------------------------------------------------------------------

/// Network database protocols that are addressed by a connection URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionProtocol {
    /// PostgreSQL (`postgres:` / `postgresql:`).
    Postgres,
    /// MySQL / MariaDB (`mysql:` / `mariadb:`).
    MySql,
}

impl ConnectionProtocol {
    /// Parses a URL protocol/scheme (case-insensitive).
    pub fn parse(raw: &str) -> Result<Self, ConnectionStringError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "mysql" | "mariadb" => Ok(Self::MySql),
            _ => Err(ConnectionStringError::UnsupportedProtocol(
                raw.trim().to_string(),
            )),
        }
    }

    /// Canonical scheme emitted by [`ConnectionString::build`].
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::MySql => "mysql",
        }
    }

    /// Port a driver uses when the URL has none.
    pub fn default_port(self) -> u16 {
        match self {
            Self::Postgres => 5432,
            Self::MySql => 3306,
        }
    }

    /// Query-parameter name for the TLS mode.
    pub fn ssl_mode_parameter(self) -> &'static str {
        match self {
            Self::Postgres => "sslmode",
            Self::MySql => "ssl-mode",
        }
    }

    /// Accepted TLS modes.
    pub fn ssl_modes(self) -> &'static [&'static str] {
        match self {
            Self::Postgres => POSTGRES_SSL_MODES,
            Self::MySql => MYSQL_SSL_MODES,
        }
    }
}

impl fmt::Display for ConnectionProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.scheme())
    }
}

/// The provider selected by an installer, including SQLite (a file, not a URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// Embedded SQLite (local file).
    Sqlite,
    /// MySQL / MariaDB.
    MySql,
    /// PostgreSQL.
    Postgres,
}

impl ProviderKind {
    /// Parses a provider name (case-insensitive; `mariadb`/`postgresql` aliases
    /// and SQLite's `sqlite3` accepted).
    pub fn parse(raw: &str) -> Result<Self, ConnectionStringError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sqlite" | "sqlite3" => Ok(Self::Sqlite),
            "mysql" | "mariadb" => Ok(Self::MySql),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            _ => Err(ConnectionStringError::UnsupportedProvider(
                raw.trim().to_string(),
            )),
        }
    }

    /// Stable upper-case provider name (e.g. for an installation marker).
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::Sqlite => "SQLITE",
            Self::MySql => "MYSQL",
            Self::Postgres => "POSTGRESQL",
        }
    }

    /// Whether the provider connects over the network (MySQL/PostgreSQL).
    pub fn is_network(self) -> bool {
        self.network_protocol().is_some()
    }

    /// URL protocol for network providers; `None` for SQLite.
    pub fn network_protocol(self) -> Option<ConnectionProtocol> {
        match self {
            Self::Sqlite => None,
            Self::MySql => Some(ConnectionProtocol::MySql),
            Self::Postgres => Some(ConnectionProtocol::Postgres),
        }
    }

    /// URL scheme for network providers; `None` for SQLite (file path).
    pub fn url_scheme(self) -> Option<&'static str> {
        self.network_protocol().map(ConnectionProtocol::scheme)
    }

    /// Default TCP port for network providers; `None` for SQLite.
    pub fn default_port(self) -> Option<u16> {
        self.network_protocol()
            .map(ConnectionProtocol::default_port)
    }

    /// Migration dialect matching this provider.
    pub fn migration_dialect(self) -> MigrationDialect {
        match self {
            Self::Sqlite => MigrationDialect::Sqlite,
            Self::MySql => MigrationDialect::MySql,
            Self::Postgres => MigrationDialect::Postgres,
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.canonical_name())
    }
}

// ---------------------------------------------------------------------------
// ConnectionString
// ---------------------------------------------------------------------------

/// A parsed database connection URL.
///
/// Format: `protocol://[userinfo@]host[:port][/database][?key=value&...]`.
/// Userinfo and query values are percent-decoded on [`Self::parse`] and
/// percent-encoded again by [`Self::build`], so special characters in a
/// password survive a round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionString {
    /// Network protocol (selects the default port and TLS vocabulary).
    pub protocol: ConnectionProtocol,
    /// User name (percent-decoded).
    pub username: Option<String>,
    /// Password (percent-decoded; `None` when the URL has no `:` userinfo).
    pub password: Option<String>,
    /// Host without IPv6 brackets.
    pub host: Option<String>,
    /// Explicit port; `None` means the driver default
    /// ([`Self::default_port`]).
    pub port: Option<u16>,
    /// Database/schema name.
    pub database: Option<String>,
    /// Query options in URL order.
    pub options: Vec<(String, String)>,
}

impl ConnectionString {
    /// Parses and validates a connection URL.
    pub fn parse(input: &str) -> Result<Self, ConnectionStringError> {
        let input = input.trim();
        let (protocol_raw, rest) = input.split_once("://").ok_or_else(|| {
            ConnectionStringError::Malformed("missing '<protocol>://' prefix".into())
        })?;
        let protocol = ConnectionProtocol::parse(protocol_raw)?;

        let (authority, query) = match rest.split_once('?') {
            Some((authority, query)) => (authority, Some(query)),
            None => (rest, None),
        };
        let (authority, database) = match authority.split_once('/') {
            Some((authority, database)) if !database.is_empty() => {
                (authority, Some(percent_decode(database)?))
            }
            Some((authority, _)) => (authority, None),
            None => (authority, None),
        };
        let (userinfo, host_port) = match authority.rsplit_once('@') {
            Some((userinfo, host_port)) => (Some(userinfo), host_port),
            None => (None, authority),
        };
        let (username, password) = match userinfo {
            Some(userinfo) => match userinfo.split_once(':') {
                Some((username, password)) => (
                    Some(percent_decode(username)?),
                    Some(percent_decode(password)?),
                ),
                None => (Some(percent_decode(userinfo)?), None),
            },
            None => (None, None),
        };

        let (host, port) = parse_host_and_port(host_port)?;
        let options = match query {
            Some(query) if !query.is_empty() => query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(|pair| {
                    let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                    Ok((percent_decode(key)?, percent_decode(value)?))
                })
                .collect::<Result<Vec<_>, ConnectionStringError>>()?,
            _ => Vec::new(),
        };

        let connection = Self {
            protocol,
            username,
            password,
            host,
            port,
            database,
            options,
        };
        connection.validate()?;
        Ok(connection)
    }

    /// Validates the semantic fields (host, port, database, TLS modes).
    pub fn validate(&self) -> Result<(), ConnectionStringError> {
        if self.port == Some(0) {
            return Err(ConnectionStringError::InvalidPort("0".into()));
        }
        if self.password.is_some() && self.username.is_none() {
            return Err(ConnectionStringError::PasswordWithoutUsername);
        }

        let host = self.host.as_deref().unwrap_or("").trim();
        if host.is_empty() {
            return Err(ConnectionStringError::Malformed("missing host".into()));
        }
        let valid_host = if host.contains(':') {
            // Bracketed IPv6 is stored without brackets.
            host.parse::<std::net::Ipv6Addr>().is_ok()
        } else {
            host.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        };
        if !valid_host {
            return Err(ConnectionStringError::InvalidHost(host.to_string()));
        }

        if let Some(database) = &self.database {
            if database.is_empty()
                || !database
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            {
                return Err(ConnectionStringError::InvalidDatabase(database.clone()));
            }
        }

        // Validate the TLS mode under whichever spelling the URL uses (both
        // spellings exist in the wild: `sslmode` for PostgreSQL/sqlx,
        // `ssl-mode` for MySQL/sqlx).
        for (key, value) in &self.options {
            let (parameter, whitelist) = if key.eq_ignore_ascii_case("sslmode") {
                ("sslmode", POSTGRES_SSL_MODES)
            } else if key.eq_ignore_ascii_case("ssl-mode") {
                ("ssl-mode", MYSQL_SSL_MODES)
            } else {
                continue;
            };
            if !whitelist
                .iter()
                .any(|mode| mode.eq_ignore_ascii_case(value))
            {
                return Err(ConnectionStringError::InvalidSslMode(
                    parameter,
                    value.clone(),
                    whitelist.join("/"),
                ));
            }
        }
        Ok(())
    }

    /// Builds the URL, percent-encoding userinfo and query values.
    ///
    /// The port is only emitted when explicitly set; use [`Self::default_port`]
    /// for "driver default" semantics.
    pub fn build(&self) -> Result<String, ConnectionStringError> {
        self.validate()?;
        let mut url = String::new();
        url.push_str(self.protocol.scheme());
        url.push_str("://");
        if let Some(username) = &self.username {
            url.push_str(&percent_encode(username));
            if let Some(password) = &self.password {
                url.push(':');
                url.push_str(&percent_encode(password));
            }
            url.push('@');
        }
        let host = self.host.as_deref().unwrap_or_default();
        if host.contains(':') {
            url.push('[');
            url.push_str(host);
            url.push(']');
        } else {
            url.push_str(host);
        }
        if let Some(port) = self.port {
            url.push(':');
            url.push_str(&port.to_string());
        }
        if let Some(database) = &self.database {
            url.push('/');
            url.push_str(database);
        }
        if !self.options.is_empty() {
            url.push('?');
            for (index, (key, value)) in self.options.iter().enumerate() {
                if index > 0 {
                    url.push('&');
                }
                url.push_str(&percent_encode(key));
                url.push('=');
                url.push_str(&percent_encode(value));
            }
        }
        Ok(url)
    }

    /// Looks up a query option by name (case-insensitive).
    pub fn option(&self, key: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    /// The explicit port or the protocol's default port.
    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.protocol.default_port())
    }
}

impl std::str::FromStr for ConnectionString {
    type Err = ConnectionStringError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

// ---------------------------------------------------------------------------
// URL primitives
// ---------------------------------------------------------------------------

fn parse_host_and_port(
    host_port: &str,
) -> Result<(Option<String>, Option<u16>), ConnectionStringError> {
    if let Some(rest) = host_port.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(|| {
            ConnectionStringError::Malformed("unterminated bracketed IPv6 host".into())
        })?;
        let port = match after {
            "" => None,
            after => match after.strip_prefix(':') {
                Some(port) if !port.is_empty() => Some(parse_port(port)?),
                _ => {
                    return Err(ConnectionStringError::Malformed(format!(
                        "unexpected text '{after}' after bracketed host"
                    )))
                }
            },
        };
        return Ok((non_empty(host), port));
    }

    // A raw `:` in an unbracketed host is the port separator; a non-numeric
    // suffix is an invalid port (typed) rather than an invalid host.
    // Unbracketed IPv6 is not a valid URL host and fails validation afterwards.
    match host_port.rsplit_once(':') {
        Some((host, port)) => Ok((non_empty(host), Some(parse_port(port)?))),
        None => Ok((non_empty(host_port), None)),
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_port(raw: &str) -> Result<u16, ConnectionStringError> {
    let port: u16 = raw
        .parse()
        .map_err(|_| ConnectionStringError::InvalidPort(raw.to_string()))?;
    if port == 0 {
        return Err(ConnectionStringError::InvalidPort(raw.to_string()));
    }
    Ok(port)
}

/// Percent-encodes everything outside RFC 3986 unreserved characters.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        let keep = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if keep {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Percent-decodes a URL component (invalid escapes are rejected).
fn percent_decode(value: &str) -> Result<String, ConnectionStringError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes
                .get(index + 1)
                .and_then(|byte| hex_value(*byte))
                .ok_or_else(|| {
                    ConnectionStringError::Malformed(format!(
                        "invalid percent-encoding in '{value}'"
                    ))
                })?;
            let low = bytes
                .get(index + 2)
                .and_then(|byte| hex_value(*byte))
                .ok_or_else(|| {
                    ConnectionStringError::Malformed(format!(
                        "invalid percent-encoding in '{value}'"
                    ))
                })?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| {
        ConnectionStringError::Malformed(format!(
            "component '{value}' is not valid UTF-8 after percent-decoding"
        ))
    })
}

fn hex_value(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|value| value as u8)
}
