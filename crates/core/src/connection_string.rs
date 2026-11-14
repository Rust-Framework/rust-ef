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
//! ADO.NET-style key/value strings (`Server=...;Uid=...;Pwd=...;Database=...`)
//! are accepted by [`ConnectionString::parse_ado`] and auto-detected by
//! [`ConnectionString::parse_auto`].
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
    /// ([`ConnectionProtocol::default_port`]).
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

    /// Parses an ADO.NET-style key/value connection string.
    ///
    /// Keys are case-insensitive; `source`/`server`/`host`/`data source`/
    /// `address`/`addr`/`network address` name the host, `uid`/`user`/`user id`/
    /// `username` the user, `pwd`/`password` the password,
    /// `database`/`initial catalog`/`db` the database and `port` the port.
    /// A port embedded in the host (`host:port`, `[IPv6]:port`) wins over the
    /// `port` key. Values may be wrapped in single or double quotes (with the
    /// quote character doubled to escape it), so a password may contain `=`,
    /// `&`, `#`, `;` and spaces. Repeated keys keep the last value.
    ///
    /// TLS keys (`sslmode`, `ssl-mode`, `ssl mode`) are normalized to the
    /// protocol's canonical spelling ([`ConnectionProtocol::ssl_mode_parameter`])
    /// and validated against [`ConnectionProtocol::ssl_modes`]; `true`/`false`
    /// map to `require`/`disable` (PostgreSQL) or `required`/`disabled` (MySQL).
    /// Any other key is kept verbatim in [`Self::options`].
    ///
    /// `protocol` selects the default port and the TLS vocabulary because an
    /// ADO.NET string carries no scheme.
    pub fn parse_ado(
        input: &str,
        protocol: ConnectionProtocol,
    ) -> Result<Self, ConnectionStringError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ConnectionStringError::Malformed(
                "empty ADO.NET connection string".into(),
            ));
        }

        let mut host_raw = None;
        let mut username = None;
        let mut password = None;
        let mut database = None;
        let mut port_raw = None;
        let mut options: Vec<(String, String)> = Vec::new();

        for segment in split_ado_segments(input)? {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            let (raw_key, raw_value) = segment.split_once('=').ok_or_else(|| {
                ConnectionStringError::Malformed(format!(
                    "malformed ADO.NET segment '{segment}': missing '='"
                ))
            })?;
            let key = raw_key.trim();
            if key.is_empty() {
                return Err(ConnectionStringError::Malformed(format!(
                    "malformed ADO.NET segment '{segment}': missing key"
                )));
            }
            let value = unquote_ado_value(raw_value)?;

            match key.to_ascii_lowercase().as_str() {
                "source" | "server" | "host" | "data source" | "address" | "addr"
                | "network address" => host_raw = non_empty_value(value),
                "uid" | "user" | "user id" | "username" => username = non_empty_value(value),
                "pwd" | "password" => password = non_empty_value(value),
                "database" | "initial catalog" | "db" => database = non_empty_value(value),
                "port" => port_raw = non_empty_value(value),
                "sslmode" | "ssl-mode" | "ssl mode" => {
                    let mode = normalize_ado_ssl_mode(protocol, &value)?;
                    let key = protocol.ssl_mode_parameter();
                    options.retain(|(existing, _)| !existing.eq_ignore_ascii_case(key));
                    options.push((key.to_string(), mode));
                }
                _ => {
                    // Unknown keys are passed through for the driver; repeated
                    // keys keep the last value, like every other field.
                    options.retain(|(existing, _)| !existing.eq_ignore_ascii_case(key));
                    options.push((key.to_string(), value));
                }
            }
        }

        let host_raw =
            host_raw.ok_or_else(|| ConnectionStringError::Malformed("missing host".into()))?;
        let (host, embedded_port) = parse_ado_host(&host_raw)?;
        let port = match (embedded_port, port_raw) {
            (Some(port), _) => Some(port),
            (None, Some(raw)) => Some(parse_port(&raw)?),
            (None, None) => None,
        };

        let connection = Self {
            protocol,
            username,
            password,
            host: Some(host),
            port,
            database,
            options,
        };
        connection.validate()?;
        Ok(connection)
    }

    /// Parses either a connection URL or an ADO.NET key/value string.
    ///
    /// Input containing `://` is handled by [`Self::parse`]; anything else is
    /// handed to [`Self::parse_ado`].
    pub fn parse_auto(
        input: &str,
        protocol: ConnectionProtocol,
    ) -> Result<Self, ConnectionStringError> {
        if input.contains("://") {
            Self::parse(input)
        } else {
            Self::parse_ado(input, protocol)
        }
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
    /// The port is only emitted when explicitly set; use [`ConnectionProtocol::default_port`]
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

// ---------------------------------------------------------------------------
// ADO.NET primitives
// ---------------------------------------------------------------------------

/// Splits an ADO.NET string into `key=value` segments on `;`.
///
/// Semicolons inside a quoted value do not split. A quote only opens a quoted
/// value when it is the first non-space character after `=`, so an apostrophe
/// inside an unquoted password is not mistaken for a quote.
fn split_ado_segments(input: &str) -> Result<Vec<&str>, ConnectionStringError> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut expect_value = false;
    let mut chars = input.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            Some(open) => {
                if ch == open {
                    if matches!(chars.peek(), Some((_, next)) if *next == open) {
                        // A doubled quote is an escaped quote, not the closing one.
                        chars.next();
                    } else {
                        quote = None;
                    }
                }
            }
            None => match ch {
                '=' if !expect_value => expect_value = true,
                '"' | '\'' if expect_value => {
                    quote = Some(ch);
                    expect_value = false;
                }
                ';' => {
                    segments.push(&input[start..index]);
                    start = index + ch.len_utf8();
                    expect_value = false;
                }
                ch if ch.is_whitespace() => {}
                _ => expect_value = false,
            },
        }
    }
    if quote.is_some() {
        return Err(ConnectionStringError::Malformed(format!(
            "unterminated quoted value in ADO.NET connection string '{input}'"
        )));
    }
    segments.push(&input[start..]);
    Ok(segments)
}

/// Trims an ADO.NET value and removes one pair of wrapping quotes, restoring
/// doubled quotes (`""` -> `"`, `''` -> `'`).
fn unquote_ado_value(value: &str) -> Result<String, ConnectionStringError> {
    let value = value.trim();
    let Some(open) = value.chars().next() else {
        return Ok(String::new());
    };
    if open != '"' && open != '\'' {
        return Ok(value.to_string());
    }
    if value.len() < 2 || !value.ends_with(open) {
        return Err(ConnectionStringError::Malformed(format!(
            "unterminated quoted value '{value}'"
        )));
    }
    let inner = &value[open.len_utf8()..value.len() - open.len_utf8()];
    let doubled = if open == '"' { "\"\"" } else { "''" };
    let replacement = if open == '"' { "\"" } else { "'" };
    Ok(inner.replace(doubled, replacement))
}

fn non_empty_value(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// Parses an ADO.NET host into an unbracketed host and an optional port.
fn parse_ado_host(raw: &str) -> Result<(String, Option<u16>), ConnectionStringError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ConnectionStringError::Malformed("missing host".into()));
    }
    if let Some(rest) = raw.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(|| {
            ConnectionStringError::Malformed("unterminated bracketed IPv6 host".into())
        })?;
        let port = match after {
            "" => None,
            after => match after.strip_prefix(':') {
                Some(port) => Some(parse_port(port)?),
                None => {
                    return Err(ConnectionStringError::Malformed(format!(
                        "unexpected text '{after}' after bracketed host"
                    )))
                }
            },
        };
        return Ok((host.to_string(), port));
    }
    // A bare IPv6 literal has several colons and no port; anything else splits
    // at the last colon. IPv6 with a port must use brackets.
    if raw.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok((raw.to_string(), None));
    }
    match raw.rsplit_once(':') {
        Some((host, port)) => Ok((host.to_string(), Some(parse_port(port)?))),
        None => Ok((raw.to_string(), None)),
    }
}

/// Validates an ADO.NET TLS value and returns the canonical spelling.
fn normalize_ado_ssl_mode(
    protocol: ConnectionProtocol,
    raw: &str,
) -> Result<String, ConnectionStringError> {
    let value = raw.trim();
    let lowered = value.to_ascii_lowercase();
    let mapped = match lowered.as_str() {
        "true" => match protocol {
            ConnectionProtocol::Postgres => "require",
            ConnectionProtocol::MySql => "required",
        },
        "false" => match protocol {
            ConnectionProtocol::Postgres => "disable",
            ConnectionProtocol::MySql => "disabled",
        },
        _ => lowered.as_str(),
    };
    let whitelist = protocol.ssl_modes();
    match whitelist
        .iter()
        .find(|mode| mode.eq_ignore_ascii_case(mapped))
    {
        Some(mode) => Ok((*mode).to_string()),
        None => Err(ConnectionStringError::InvalidSslMode(
            protocol.ssl_mode_parameter(),
            value.to_string(),
            whitelist.join("/"),
        )),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ado(input: &str) -> ConnectionString {
        ConnectionString::parse_ado(input, ConnectionProtocol::Postgres)
            .unwrap_or_else(|err| panic!("'{input}' must parse: {err}"))
    }

    fn parse_ado_err(input: &str) -> ConnectionStringError {
        ConnectionString::parse_ado(input, ConnectionProtocol::Postgres).unwrap_err()
    }

    /// `parse_ado(..).build()` must produce a URL that `parse` maps back to the
    /// exact same fields.
    fn assert_url_round_trip(cs: &ConnectionString) {
        let url = cs.build().expect("build must succeed");
        let reparsed = ConnectionString::parse(&url)
            .unwrap_or_else(|err| panic!("built URL '{url}' must parse: {err}"));
        assert_eq!(
            &reparsed, cs,
            "ADO -> build -> parse must preserve every field"
        );
    }

    #[test]
    fn ado_host_aliases_port_separation_and_default_port() {
        for alias in [
            "source",
            "server",
            "host",
            "data source",
            "address",
            "addr",
            "network address",
        ] {
            for key in [alias.to_string(), alias.to_ascii_uppercase()] {
                let cs = parse_ado(&format!(
                    "{key}=db.internal;uid=app;pwd=secret;database=travel"
                ));
                assert_eq!(cs.protocol, ConnectionProtocol::Postgres);
                assert_eq!(cs.host.as_deref(), Some("db.internal"), "{key}");
                assert_eq!(cs.username.as_deref(), Some("app"), "{key}");
                assert_eq!(cs.password.as_deref(), Some("secret"), "{key}");
                assert_eq!(cs.database.as_deref(), Some("travel"), "{key}");
                assert_eq!(cs.port, None, "{key}: no explicit port");
                assert_eq!(cs.effective_port(), 5432, "{key}: default Postgres port");
                assert_url_round_trip(&cs);
            }
        }

        // The separate `port` key is used when the host has none.
        let cs = parse_ado("Server=db.internal;Port=6543;Uid=app;Pwd=secret");
        assert_eq!(cs.port, Some(6543));
        assert_eq!(cs.effective_port(), 6543);

        // A port embedded in the host wins over the `port` key.
        let cs = parse_ado("Server=db.internal:6432;Port=7000;Uid=app;Pwd=secret");
        assert_eq!(cs.host.as_deref(), Some("db.internal"));
        assert_eq!(cs.port, Some(6432));
    }

    #[test]
    fn ado_ipv6_hosts_are_supported() {
        let cs = parse_ado("Server=[::1]:6543;Uid=app;Pwd=secret;Database=travel");
        assert_eq!(cs.host.as_deref(), Some("::1"));
        assert_eq!(cs.port, Some(6543));
        assert_url_round_trip(&cs);

        let cs = parse_ado("Server=[::1];Uid=app;Pwd=secret");
        assert_eq!(cs.host.as_deref(), Some("::1"));
        assert_eq!(cs.port, None);
        assert_url_round_trip(&cs);
    }

    #[test]
    fn ado_tolerates_trailing_and_repeated_semicolons() {
        for input in [
            "source=db;uid=u;pwd=p;",
            ";;source=db;;uid=u;;pwd=p;;",
            "  source=db ; uid=u ; pwd=p ; ",
        ] {
            let cs = parse_ado(input);
            assert_eq!(cs.host.as_deref(), Some("db"), "{input}");
            assert_eq!(cs.username.as_deref(), Some("u"), "{input}");
            assert_eq!(cs.password.as_deref(), Some("p"), "{input}");
        }

        // Repeated keys keep the last value.
        let cs = parse_ado("source=a;source=db;uid=u;uid=app;pwd=p;pwd=secret;");
        assert_eq!(cs.host.as_deref(), Some("db"));
        assert_eq!(cs.username.as_deref(), Some("app"));
        assert_eq!(cs.password.as_deref(), Some("secret"));
    }

    #[test]
    fn ado_password_preserves_special_characters() {
        // Unquoted: `=`, `&`, `#` and spaces belong to the value; only the
        // first `=` of the segment separates the key.
        let cs = parse_ado("Server=db;Uid=app;Pwd=p=a&s#s word");
        assert_eq!(cs.password.as_deref(), Some("p=a&s#s word"));
        assert_url_round_trip(&cs);

        // Semicolons need quoting, which is split-aware and unquoted on parse.
        let cs = parse_ado("Server=db;Uid=app;Pwd=\"a;b=c&d#e f\"");
        assert_eq!(cs.password.as_deref(), Some("a;b=c&d#e f"));
        assert_url_round_trip(&cs);

        // Doubled quotes inside a quoted value are restored.
        let cs = parse_ado("Server=db;Uid=app;Pwd=\"a\"\"b;c\"");
        assert_eq!(cs.password.as_deref(), Some("a\"b;c"));
        assert_url_round_trip(&cs);

        let cs = parse_ado("Server=db;Uid=app;Pwd='it''s;fine'");
        assert_eq!(cs.password.as_deref(), Some("it's;fine"));
        assert_url_round_trip(&cs);
    }

    #[test]
    fn ado_malformed_segments_and_missing_host_are_typed() {
        let err = parse_ado_err("Server=db;uid");
        match err {
            ConnectionStringError::Malformed(message) => {
                assert!(
                    message.contains("uid"),
                    "message must name the segment: {message}"
                );
                assert!(message.contains("missing '='"), "message: {message}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }

        for input in [
            "",
            "   ",
            ";;",
            "  ;;  ",
            "uid=app;pwd=secret",
            "Application Name=x",
        ] {
            assert!(
                matches!(
                    ConnectionString::parse_ado(input, ConnectionProtocol::Postgres),
                    Err(ConnectionStringError::Malformed(_))
                ),
                "'{input}' must be a typed Malformed error"
            );
        }

        // Empty values are tolerated and treated as absent.
        let cs = parse_ado("Server=db;Uid=;Pwd=;Database=;Port=;");
        assert_eq!(cs.username, None);
        assert_eq!(cs.password, None);
        assert_eq!(cs.database, None);
        assert_eq!(cs.port, None);
        assert_url_round_trip(&cs);
    }

    #[test]
    fn ado_invalid_ports_are_typed() {
        for input in [
            "Server=db:0;Uid=app;Pwd=secret",
            "Server=db:65536;Uid=app;Pwd=secret",
            "Server=db:notaport;Uid=app;Pwd=secret",
            "Server=[::1]:0;Uid=app;Pwd=secret",
            "Server=[::1]:abc;Uid=app;Pwd=secret",
            "Server=db;Port=0;Uid=app;Pwd=secret",
            "Server=db;Port=abc;Uid=app;Pwd=secret",
        ] {
            let err = ConnectionString::parse_ado(input, ConnectionProtocol::Postgres).unwrap_err();
            assert!(
                matches!(err, ConnectionStringError::InvalidPort(_)),
                "{input}: expected InvalidPort, got {err:?}"
            );
        }
    }

    #[test]
    fn ado_tls_aliases_are_normalized() {
        let cs = parse_ado("Server=db;Uid=app;Pwd=secret;sslmode=VERIFY-FULL");
        assert_eq!(cs.option("sslmode"), Some("verify-full"));
        assert_eq!(
            cs.options,
            vec![("sslmode".to_string(), "verify-full".to_string())]
        );

        // `ssl-mode` / `ssl mode` become the canonical Postgres key.
        for input in [
            "Server=db;Uid=app;Pwd=secret;ssl-mode=Require",
            "Server=db;Uid=app;Pwd=secret;SSL MODE=require",
        ] {
            let cs = parse_ado(input);
            assert_eq!(cs.option("sslmode"), Some("require"), "{input}");
        }

        // `SSL_MODE` is not an alias; it stays an unknown option untouched.
        let cs = parse_ado("Server=db;Uid=app;Pwd=secret;SSL_MODE=require");
        assert_eq!(cs.option("SSL_MODE"), Some("require"));
        assert_eq!(cs.options.len(), 1);

        // Booleans map to each protocol's vocabulary.
        let cs = parse_ado("Server=db;Uid=app;Pwd=secret;ssl mode=true");
        assert_eq!(cs.option("sslmode"), Some("require"));
        let cs = parse_ado("Server=db;Uid=app;Pwd=secret;sslmode=false");
        assert_eq!(cs.option("sslmode"), Some("disable"));

        let cs = ConnectionString::parse_ado(
            "Server=db;Uid=app;Pwd=secret;ssl-mode=TRUE",
            ConnectionProtocol::MySql,
        )
        .unwrap();
        assert_eq!(
            cs.options,
            vec![("ssl-mode".to_string(), "required".to_string())]
        );
        let cs = ConnectionString::parse_ado(
            "Server=db;Uid=app;Pwd=secret;SSL Mode=False",
            ConnectionProtocol::MySql,
        )
        .unwrap();
        assert_eq!(cs.option("ssl-mode"), Some("disabled"));
        let cs = ConnectionString::parse_ado(
            "Server=db;Uid=app;Pwd=secret;ssl mode=verify_identity",
            ConnectionProtocol::MySql,
        )
        .unwrap();
        assert_eq!(cs.option("ssl-mode"), Some("verify_identity"));
        assert_url_round_trip(&cs);

        // The last TLS alias wins.
        let cs = parse_ado("Server=db;Uid=app;Pwd=secret;sslmode=require;ssl mode=disable");
        assert_eq!(
            cs.options,
            vec![("sslmode".to_string(), "disable".to_string())]
        );
    }

    #[test]
    fn ado_tls_rejects_unknown_modes() {
        let err = parse_ado_err("Server=db;Uid=app;Pwd=secret;sslmode=on");
        assert!(
            matches!(
                err,
                ConnectionStringError::InvalidSslMode("sslmode", ref value, _) if value == "on"
            ),
            "unexpected error: {err:?}"
        );

        let err = ConnectionString::parse_ado(
            "Server=db;Uid=app;Pwd=secret;ssl mode=require",
            ConnectionProtocol::MySql,
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                ConnectionStringError::InvalidSslMode("ssl-mode", ref value, _)
                    if value == "require"
            ),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn ado_unknown_keys_are_preserved_as_options() {
        let cs = parse_ado(
            "Server=db;Uid=app;Pwd=secret;Application Name=Travel App;Connect Timeout=30",
        );
        assert_eq!(cs.host.as_deref(), Some("db"));
        assert_eq!(cs.username.as_deref(), Some("app"));
        assert_eq!(cs.option("Application Name"), Some("Travel App"));
        assert_eq!(cs.option("connect timeout"), Some("30"));
        assert_eq!(cs.options.len(), 2);
        assert_url_round_trip(&cs);
    }

    #[test]
    fn ado_build_round_trips_through_the_url_parser() {
        let cs = ConnectionString::parse_ado(
            "Server=db.internal:6543;Uid=user name;Pwd=p@ss:w/rd=a&b#c \"q\" 'z';Database=travel;SSL Mode=true;Application Name=Travel App",
            ConnectionProtocol::Postgres,
        )
        .unwrap();
        assert_eq!(cs.host.as_deref(), Some("db.internal"));
        assert_eq!(cs.port, Some(6543));
        assert_eq!(cs.username.as_deref(), Some("user name"));
        assert_eq!(cs.password.as_deref(), Some("p@ss:w/rd=a&b#c \"q\" 'z'"));
        assert_eq!(cs.option("sslmode"), Some("require"));
        assert_url_round_trip(&cs);
    }

    #[test]
    fn parse_auto_detects_urls_and_ado_strings() {
        let url = ConnectionString::parse_auto(
            "postgres://user:pw@db:5432/travel",
            ConnectionProtocol::Postgres,
        )
        .expect("URL");
        assert_eq!(url.host.as_deref(), Some("db"));
        assert_eq!(url.port, Some(5432));

        let ado = ConnectionString::parse_auto(
            "Server=db;Uid=user;Pwd=pw;Database=travel",
            ConnectionProtocol::Postgres,
        )
        .expect("ADO");
        assert_eq!(ado.host.as_deref(), Some("db"));
        assert_eq!(ado.username.as_deref(), Some("user"));

        assert!(matches!(
            ConnectionString::parse_auto("not-a-url", ConnectionProtocol::Postgres),
            Err(ConnectionStringError::Malformed(_))
        ));
        // `parse_auto` keeps rejecting malformed URLs through `parse`.
        assert!(matches!(
            ConnectionString::parse_auto(
                "oracle://user:pw@db:1521/svc",
                ConnectionProtocol::Postgres
            ),
            Err(ConnectionStringError::UnsupportedProtocol(_))
        ));
    }
}
