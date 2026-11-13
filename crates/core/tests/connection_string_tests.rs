//! Connection-string parsing/building/validation (EF-15).

use rust_ef::connection_string::{
    ConnectionProtocol, ConnectionString, ConnectionStringError, ProviderKind, MYSQL_SSL_MODES,
    POSTGRES_SSL_MODES,
};

fn network(protocol: ConnectionProtocol) -> ConnectionString {
    ConnectionString {
        protocol,
        username: Some("travel".into()),
        password: Some("p@ss word".into()),
        host: Some("127.0.0.1".into()),
        port: Some(55432),
        database: Some("travel_test".into()),
        options: Vec::new(),
    }
}

#[test]
fn special_character_password_round_trips() {
    let mut cs = network(ConnectionProtocol::Postgres);
    cs.password = Some("p@ss:w/rd %?#&+".into());
    cs.username = Some("user name".into());

    let url = cs.build().expect("build");
    assert_eq!(
        url,
        "postgres://user%20name:p%40ss%3Aw%2Frd%20%25%3F%23%26%2B@127.0.0.1:55432/travel_test"
    );

    let parsed = ConnectionString::parse(&url).expect("parse");
    assert_eq!(parsed, cs, "build -> parse must preserve every field");
}

#[test]
fn build_encodes_userinfo_and_keeps_protocol_canonical() {
    let mut cs = network(ConnectionProtocol::MySql);
    cs.port = None;
    let url = cs.build().unwrap();
    assert_eq!(url, "mysql://travel:p%40ss%20word@127.0.0.1/travel_test");

    // Aliases parse to the canonical protocol; the explicit port survives.
    let parsed = ConnectionString::parse(
        "mariadb://travel:secret@db.internal:3306/travel?ssl-mode=required",
    )
    .unwrap();
    assert_eq!(parsed.protocol, ConnectionProtocol::MySql);
    assert_eq!(parsed.effective_port(), 3306);
    assert_eq!(parsed.option("ssl-mode"), Some("required"));
    assert_eq!(
        parsed.build().unwrap(),
        "mysql://travel:secret@db.internal:3306/travel?ssl-mode=required"
    );
}

#[test]
fn unsupported_protocol_is_a_typed_error() {
    let err = ConnectionString::parse("oracle://user:pw@db:1521/svc").unwrap_err();
    assert_eq!(
        err,
        ConnectionStringError::UnsupportedProtocol("oracle".into())
    );
    assert!(matches!(
        ConnectionString::parse("not-a-url"),
        Err(ConnectionStringError::Malformed(_))
    ));
}

#[test]
fn invalid_ports_are_typed_errors() {
    for raw in [
        "postgres://u:p@host:0/db",
        "postgres://u:p@host:65536/db",
        "postgres://u:p@host:notaport/db",
    ] {
        let err = ConnectionString::parse(raw).unwrap_err();
        assert!(
            matches!(err, ConnectionStringError::InvalidPort(_)),
            "{raw}: expected InvalidPort, got {err:?}"
        );
    }

    // A struct constructed without parsing rejects port 0 in both validate and
    // build; `build` never emits an invalid URL.
    let mut cs = network(ConnectionProtocol::Postgres);
    cs.port = Some(0);
    assert!(matches!(
        cs.validate(),
        Err(ConnectionStringError::InvalidPort(_))
    ));
    assert!(cs.build().is_err());
}

#[test]
fn postgres_sslmode_is_whitelisted() {
    for mode in POSTGRES_SSL_MODES {
        let url = format!("postgres://u:p@db:5432/travel?sslmode={mode}");
        let cs = ConnectionString::parse(&url)
            .unwrap_or_else(|err| panic!("sslmode={mode} must be accepted: {err}"));
        assert_eq!(cs.option("sslmode"), Some(*mode));
    }

    let err = ConnectionString::parse("postgres://u:p@db:5432/travel?sslmode=disabled").unwrap_err();
    assert!(
        matches!(
            err,
            ConnectionStringError::InvalidSslMode("sslmode", ref value, _)
                if value == "disabled"
        ),
        "unexpected error: {err:?}"
    );
}

#[test]
fn mysql_ssl_mode_is_whitelisted() {
    for mode in MYSQL_SSL_MODES {
        let url = format!("mysql://u:p@db:3306/travel?ssl-mode={mode}");
        ConnectionString::parse(&url)
            .unwrap_or_else(|err| panic!("ssl-mode={mode} must be accepted: {err}"));
    }

    let err = ConnectionString::parse("mysql://u:p@db:3306/travel?ssl-mode=require").unwrap_err();
    assert!(
        matches!(
            err,
            ConnectionStringError::InvalidSslMode("ssl-mode", ref value, _) if value == "require"
        ),
        "unexpected error: {err:?}"
    );
}

#[test]
fn ipv6_hosts_are_bracketed() {
    let cs =
        ConnectionString::parse("postgres://app:secret@[::1]:5432/travel").expect("parse IPv6");
    assert_eq!(cs.host.as_deref(), Some("::1"));
    assert_eq!(cs.port, Some(5432));
    assert_eq!(
        cs.build().unwrap(),
        "postgres://app:secret@[::1]:5432/travel"
    );

    let err = ConnectionString::parse("postgres://app@[::1/travel").unwrap_err();
    assert!(matches!(err, ConnectionStringError::Malformed(_)));
}

#[test]
fn hosts_and_database_names_are_validated() {
    let mut cs = network(ConnectionProtocol::Postgres);
    cs.host = Some("bad host".into());
    assert!(matches!(
        cs.validate(),
        Err(ConnectionStringError::InvalidHost(_))
    ));

    let mut cs = network(ConnectionProtocol::Postgres);
    cs.host = Some(String::new());
    assert!(cs.validate().is_err(), "a network URL needs a host");

    let mut cs = network(ConnectionProtocol::Postgres);
    cs.database = Some("bad/db".into());
    assert!(matches!(
        cs.validate(),
        Err(ConnectionStringError::InvalidDatabase(_))
    ));
}

#[test]
fn a_password_without_a_username_is_rejected() {
    let cs = ConnectionString {
        protocol: ConnectionProtocol::Postgres,
        username: None,
        password: Some("secret".into()),
        host: Some("db".into()),
        port: None,
        database: Some("travel".into()),
        options: Vec::new(),
    };
    assert_eq!(
        cs.validate().unwrap_err(),
        ConnectionStringError::PasswordWithoutUsername
    );
}

#[test]
fn connection_string_errors_convert_to_typed_ef_errors() {
    let err: rust_ef::error::EFError = ConnectionStringError::UnsupportedProtocol("oracle".into())
        .into();
    assert!(err.message().contains("oracle"));
    // The typed cause stays reachable through the error chain.
    let source = std::error::Error::source(&err).expect("source");
    assert!(source.downcast_ref::<ConnectionStringError>().is_some());
}

#[test]
fn provider_kind_maps_names_schemes_and_dialects() {
    assert_eq!(ProviderKind::parse("sqlite").unwrap(), ProviderKind::Sqlite);
    assert_eq!(
        ProviderKind::parse("POSTGRESQL").unwrap(),
        ProviderKind::Postgres
    );
    assert_eq!(ProviderKind::parse("mariadb").unwrap().canonical_name(), "MYSQL");
    assert_eq!(ProviderKind::parse("postgresql").unwrap().canonical_name(), "POSTGRESQL");
    assert_eq!(
        ProviderKind::Postgres.url_scheme(),
        Some(ConnectionProtocol::Postgres.scheme())
    );
    assert_eq!(ProviderKind::MySql.url_scheme(), Some("mysql"));
    assert_eq!(ProviderKind::Sqlite.url_scheme(), None);
    assert!(!ProviderKind::Sqlite.is_network());
    assert!(ProviderKind::MySql.is_network());
    assert_eq!(ProviderKind::Postgres.default_port(), Some(5432));
    assert_eq!(ProviderKind::MySql.default_port(), Some(3306));
    assert_eq!(ProviderKind::Sqlite.default_port(), None);
    assert_eq!(
        ProviderKind::Postgres.migration_dialect(),
        rust_ef::migration::MigrationDialect::Postgres
    );
    assert_eq!(
        ProviderKind::Sqlite.migration_dialect(),
        rust_ef::migration::MigrationDialect::Sqlite
    );
    assert_eq!(ProviderKind::MySql.to_string(), "MYSQL");

    let err = ProviderKind::parse("oracle").unwrap_err();
    assert_eq!(
        err,
        ConnectionStringError::UnsupportedProvider("oracle".into())
    );
}
