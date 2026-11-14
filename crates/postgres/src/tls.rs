/// TLS mode for PostgreSQL connections.
///
/// `Disable` uses plaintext connections (`tokio_postgres::NoTls`) — the
/// pre-v1.4 default, kept for backward compatibility.
///
/// `Require` enforces TLS via the platform's native TLS implementation
/// (SChannel on Windows, OpenSSL on Linux, Secure Transport on macOS). The
/// connector is cloned per pool acquisition, so it must be `Clone`
/// (`native_tls::TlsConnector` satisfies this).
#[derive(Clone)]
pub enum PgTlsMode {
    /// Plaintext connection (backward compatible with v1.3).
    Disable,
    /// Enforce TLS using the provided `native_tls::TlsConnector`.
    Require(native_tls::TlsConnector),
}

/// TLS preference expressed by a PostgreSQL connection string.
///
/// Mirrors the `sslmode` vocabulary of libpq / `ConnectionString`, but only the
/// two outcomes this provider can enforce without a fallback handshake:
/// plaintext or TLS-required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgTlsPreference {
    /// `sslmode=disable` or `sslmode=allow`: plaintext is explicitly allowed.
    Plaintext,
    /// `sslmode=require` / `verify-ca` / `verify-full` / `prefer`, or no
    /// `sslmode` at all: TLS is required (secure by default).
    Encrypted,
}

impl PgTlsPreference {
    /// Whether this preference requires a TLS connector.
    pub fn is_encrypted(self) -> bool {
        matches!(self, Self::Encrypted)
    }
}

/// Reads the `sslmode` query parameter from a PostgreSQL URL.
///
/// The provider does not try TLS and fall back: `prefer` (and an absent
/// parameter) therefore means "encrypted" so a connection can never silently
/// downgrade. `allow` explicitly permits plaintext and maps to
/// [`PgTlsPreference::Plaintext`].
///
/// Invalid values are ignored here; [`rust_ef::connection_string::ConnectionString`]
/// is the validating parser and returns a typed error for them.
pub fn pg_tls_preference(connection_string: &str) -> PgTlsPreference {
    let Some(query) = connection_string.split_once('?').map(|(_, query)| query) else {
        return PgTlsPreference::Encrypted;
    };
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if !key.eq_ignore_ascii_case("sslmode") {
            continue;
        }
        let value = percent_decode(value);
        return match value.trim().to_ascii_lowercase().as_str() {
            "disable" | "allow" => PgTlsPreference::Plaintext,
            _ => PgTlsPreference::Encrypted,
        };
    }
    PgTlsPreference::Encrypted
}

/// Removes the `sslmode` query parameter before handing a URL to
/// `tokio_postgres`.
///
/// `tokio_postgres::Config` only understands `disable`/`prefer`/`require`; it
/// rejects `verify-ca`/`verify-full`, which `ConnectionString` accepts. The
/// provider decides the TLS mode itself ([`pg_tls_preference`]), so the
/// parameter is stripped from the driver URL.
pub(crate) fn without_ssl_mode_parameter(connection_string: &str) -> String {
    let Some((base, query)) = connection_string.split_once('?') else {
        return connection_string.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            pair.split_once('=')
                .map(|(key, _)| !key.eq_ignore_ascii_case("sslmode"))
                .unwrap_or(true)
        })
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

/// Minimal percent-decoding for URL query values (invalid escapes are kept).
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{pg_tls_preference, PgTlsPreference};

    #[test]
    fn missing_sslmode_is_encrypted() {
        assert_eq!(
            pg_tls_preference("postgres://user:pass@host:5432/db"),
            PgTlsPreference::Encrypted
        );
    }

    #[test]
    fn disable_and_allow_are_plaintext() {
        assert_eq!(
            pg_tls_preference("postgres://host/db?sslmode=disable"),
            PgTlsPreference::Plaintext
        );
        assert_eq!(
            pg_tls_preference("postgres://host/db?sslmode=allow"),
            PgTlsPreference::Plaintext
        );
    }

    #[test]
    fn require_and_prefer_are_encrypted() {
        for mode in ["require", "prefer", "verify-full", "verify ca"] {
            assert_eq!(
                pg_tls_preference(&format!("postgres://host/db?sslmode={mode}")),
                PgTlsPreference::Encrypted,
                "mode {mode}"
            );
        }
    }

    #[test]
    fn parameter_lookup_is_case_insensitive_and_handles_other_options() {
        assert_eq!(
            pg_tls_preference("postgres://host/db?application_name=app&SSLMODE=Disable"),
            PgTlsPreference::Plaintext
        );
    }

    #[test]
    fn sslmode_is_removed_before_driver_parsing() {
        assert_eq!(
            super::without_ssl_mode_parameter("postgres://host/db?sslmode=verify-full"),
            "postgres://host/db"
        );
        assert_eq!(
            super::without_ssl_mode_parameter(
                "postgres://host/db?application_name=app&sslmode=disable&connect_timeout=5"
            ),
            "postgres://host/db?application_name=app&connect_timeout=5"
        );
        assert_eq!(
            super::without_ssl_mode_parameter("postgres://host/db"),
            "postgres://host/db"
        );
    }
}
