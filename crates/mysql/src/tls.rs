/// TLS mode for MySQL connections.
///
/// Unlike `PgTlsMode`, this enum does not carry a `TlsConnector` — sqlx
/// manages TLS internally via its `tls-native-tls` feature. CA certificates
/// are configured via the connection string's `ssl-ca` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MySqlTlsMode {
    /// Disable TLS — plaintext connections only.
    Disabled,
    /// Require TLS — connection fails if the server doesn't support TLS.
    Required,
    /// Require TLS with CA certificate verification.
    VerifyCa,
    /// Require TLS with CA certificate and hostname verification.
    VerifyIdentity,
}

impl From<MySqlTlsMode> for sqlx::mysql::MySqlSslMode {
    fn from(mode: MySqlTlsMode) -> Self {
        match mode {
            MySqlTlsMode::Disabled => Self::Disabled,
            MySqlTlsMode::Required => Self::Required,
            MySqlTlsMode::VerifyCa => Self::VerifyCa,
            MySqlTlsMode::VerifyIdentity => Self::VerifyIdentity,
        }
    }
}
