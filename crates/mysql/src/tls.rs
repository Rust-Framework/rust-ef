/// TLS mode for MySQL connections.
///
/// Unlike `PgTlsMode`, this enum does not carry a `TlsConnector` — sqlx
/// manages TLS internally via its `tls-native-tls` feature. CA certificates
/// are configured via the connection string's `ssl-ca` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MySqlTlsMode {
    /// Disable TLS — plaintext connections only.
    Disabled,
    /// Use TLS when the server supports it, plaintext otherwise. This is
    /// sqlx's own default and is only selected when the connection string asks
    /// for it explicitly (`ssl-mode=preferred`).
    Preferred,
    /// Require TLS — connection fails if the server doesn't support TLS.
    Required,
    /// Require TLS with CA certificate verification.
    VerifyCa,
    /// Require TLS with CA certificate and hostname verification.
    VerifyIdentity,
}

impl From<MySqlTlsMode> for sqlx_mysql::MySqlSslMode {
    fn from(mode: MySqlTlsMode) -> Self {
        match mode {
            MySqlTlsMode::Disabled => Self::Disabled,
            MySqlTlsMode::Preferred => Self::Preferred,
            MySqlTlsMode::Required => Self::Required,
            MySqlTlsMode::VerifyCa => Self::VerifyCa,
            MySqlTlsMode::VerifyIdentity => Self::VerifyIdentity,
        }
    }
}

/// Reads an explicit `ssl-mode` from a MySQL URL.
///
/// Returns `None` when the URL does not mention TLS at all, so the provider can
/// apply its secure default (`Required`) instead of sqlx's `Preferred`. Both
/// spellings used in the wild (`ssl-mode` and `sslmode`) are accepted, matching
/// `ConnectionString` validation.
pub fn tls_mode_from_connection_string(connection_string: &str) -> Option<MySqlTlsMode> {
    let query = connection_string.split_once('?').map(|(_, query)| query)?;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if !key.eq_ignore_ascii_case("ssl-mode") && !key.eq_ignore_ascii_case("sslmode") {
            continue;
        }
        return Some(match value.trim().to_ascii_lowercase().as_str() {
            "disabled" | "disable" | "none" | "false" | "0" => MySqlTlsMode::Disabled,
            "preferred" | "prefer" => MySqlTlsMode::Preferred,
            "verify_ca" | "verify-ca" => MySqlTlsMode::VerifyCa,
            "verify_identity" | "verify-identity" => MySqlTlsMode::VerifyIdentity,
            // `required`/`require`/`true` and unknown values stay secure.
            _ => MySqlTlsMode::Required,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{tls_mode_from_connection_string, MySqlTlsMode};

    #[test]
    fn absent_parameter_means_use_provider_default() {
        assert_eq!(
            tls_mode_from_connection_string("mysql://user:pass@host:3306/db"),
            None
        );
    }

    #[test]
    fn explicit_modes_are_honored() {
        assert_eq!(
            tls_mode_from_connection_string("mysql://host/db?ssl-mode=disabled"),
            Some(MySqlTlsMode::Disabled)
        );
        assert_eq!(
            tls_mode_from_connection_string("mysql://host/db?sslmode=verify-ca"),
            Some(MySqlTlsMode::VerifyCa)
        );
        assert_eq!(
            tls_mode_from_connection_string("mysql://host/db?SSL-MODE=REQUIRED"),
            Some(MySqlTlsMode::Required)
        );
    }

    #[test]
    fn unknown_mode_stays_secure() {
        assert_eq!(
            tls_mode_from_connection_string("mysql://host/db?ssl-mode=surprise"),
            Some(MySqlTlsMode::Required)
        );
    }
}
