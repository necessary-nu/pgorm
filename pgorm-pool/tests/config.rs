//! `Config` on its own: what reaches the `pgorm_driver::Config` it builds,
//! and what its `Debug` refuses to print. Neither question needs a server, so
//! nothing here connects.

use pgorm_driver::config::{
    ChannelBinding as PgChannelBinding, LoadBalanceHosts as PgLoadBalanceHosts,
    SslMode as PgSslMode, TargetSessionAttrs as PgTargetSessionAttrs,
};
use pgorm_pool::{ChannelBinding, Config, LoadBalanceHosts, SslMode, TargetSessionAttrs};

/// The minimum a `Config` needs to build: `get_pg_config` rejects a missing
/// database name before it looks at anything else.
fn named() -> Config {
    Config {
        dbname: Some("example".to_owned()),
        ..Default::default()
    }
}

// [spec:pgorm:req:conn.pool.config-forwarding+2/test]    target_session_attrs reaches the connection
#[test]
fn target_session_attrs_is_forwarded() {
    let pg = named().get_pg_config().expect("a named config builds");
    assert_eq!(
        pg.get_target_session_attrs(),
        PgTargetSessionAttrs::Any,
        "an unset field leaves tokio-postgres on its own default"
    );

    let pg = Config {
        target_session_attrs: Some(TargetSessionAttrs::ReadWrite),
        ..named()
    }
    .get_pg_config()
    .expect("a named config builds");

    assert_eq!(
        pg.get_target_session_attrs(),
        PgTargetSessionAttrs::ReadWrite,
        "ReadWrite dropped here lands writes on a hot standby"
    );
}

// [spec:pgorm:req:conn.pool.config-forwarding+2/test]    channel_binding reaches the connection
#[test]
fn channel_binding_is_forwarded() {
    let pg = named().get_pg_config().expect("a named config builds");
    assert_eq!(pg.get_channel_binding(), PgChannelBinding::Prefer);

    for (ours, theirs) in [
        (ChannelBinding::Disable, PgChannelBinding::Disable),
        (ChannelBinding::Prefer, PgChannelBinding::Prefer),
        (ChannelBinding::Require, PgChannelBinding::Require),
    ] {
        let pg = Config {
            channel_binding: Some(ours),
            ..named()
        }
        .get_pg_config()
        .expect("a named config builds");

        assert_eq!(
            pg.get_channel_binding(),
            theirs,
            "{ours:?} dropped here is a silently weaker connection"
        );
    }
}

// [spec:pgorm:req:conn.pool.config-forwarding+2/test]    load_balance_hosts reaches the connection
#[test]
fn load_balance_hosts_is_forwarded() {
    let pg = named().get_pg_config().expect("a named config builds");
    assert_eq!(pg.get_load_balance_hosts(), PgLoadBalanceHosts::Disable);

    let pg = Config {
        load_balance_hosts: Some(LoadBalanceHosts::Random),
        ..named()
    }
    .get_pg_config()
    .expect("a named config builds");

    assert_eq!(pg.get_load_balance_hosts(), PgLoadBalanceHosts::Random);
}

// [spec:pgorm:req:conn.pool.config-forwarding+2/test]    ssl_mode, which was already forwarded, still is
#[test]
fn ssl_mode_is_forwarded() {
    let pg = Config {
        ssl_mode: Some(SslMode::Require),
        ..named()
    }
    .get_pg_config()
    .expect("a named config builds");

    assert_eq!(pg.get_ssl_mode(), PgSslMode::Require);
}

// [spec:pgorm:req:conn.pool.config-forwarding+2/test]    the struct form and the URL form agree
#[test]
fn struct_form_matches_url_form() {
    let from_url = Config {
        url: Some(
            "postgres://localhost/example\
             ?target_session_attrs=read-write&channel_binding=require&load_balance_hosts=random"
                .to_owned(),
        ),
        ..Default::default()
    }
    .get_pg_config()
    .expect("the URL carries a database name");

    let from_struct = Config {
        url: Some("postgres://localhost/example".to_owned()),
        target_session_attrs: Some(TargetSessionAttrs::ReadWrite),
        channel_binding: Some(ChannelBinding::Require),
        load_balance_hosts: Some(LoadBalanceHosts::Random),
        ..Default::default()
    }
    .get_pg_config()
    .expect("the URL carries a database name");

    assert_eq!(
        from_struct.get_target_session_attrs(),
        from_url.get_target_session_attrs()
    );
    assert_eq!(
        from_struct.get_channel_binding(),
        from_url.get_channel_binding()
    );
    assert_eq!(
        from_struct.get_load_balance_hosts(),
        from_url.get_load_balance_hosts()
    );
}

// [spec:pgorm:sem:conn.pool.config-redaction+1/test]    neither credential survives Debug
#[test]
fn debug_reveals_no_credential() {
    let cfg = Config {
        url: Some("postgres://john_doe:topsecret@pg.example.com:5432/example".to_owned()),
        user: Some("john_doe".to_owned()),
        password: Some("topsecret".to_owned()),
        ..named()
    };

    let printed = format!("{cfg:?}");

    assert!(
        !printed.contains("topsecret"),
        "the password reached the log: {printed}"
    );
    assert!(
        !printed.contains("john_doe:"),
        "the URL's userinfo reached the log: {printed}"
    );
    assert!(
        printed.contains("password: Some(_)"),
        "a set password is marked present but withheld: {printed}"
    );
    assert!(
        printed.contains("postgres://_@pg.example.com:5432/example"),
        "everything after the userinfo is what makes the URL worth printing: {printed}"
    );
}

// [spec:pgorm:sem:conn.pool.config-redaction+1/test]    an absent password is absent, not withheld
#[test]
fn debug_distinguishes_unset_from_withheld() {
    let printed = format!("{:?}", named());

    assert!(printed.contains("password: None"), "{printed}");
    assert!(printed.contains("url: None"), "{printed}");
}

// [spec:pgorm:sem:conn.pool.config-redaction+1/test]    a URL with no credentials is printed whole
#[test]
fn debug_keeps_a_credential_free_url() {
    let cfg = Config {
        url: Some("postgres://pg.example.com:5432/example?sslmode=require".to_owned()),
        ..Default::default()
    };

    assert!(
        format!("{cfg:?}").contains("postgres://pg.example.com:5432/example?sslmode=require"),
        "nothing was hidden, so nothing is cut"
    );
}

// [spec:pgorm:sem:conn.pool.config-redaction+1/test]    a keyword/value URL that names a password is withheld whole
#[test]
fn debug_withholds_a_keyword_value_url() {
    let cfg = Config {
        url: Some("host=pg.example.com dbname=example password=topsecret".to_owned()),
        ..Default::default()
    };

    let printed = format!("{cfg:?}");

    assert!(!printed.contains("topsecret"), "{printed}");
    assert!(printed.contains("url: Some(\"_\")"), "{printed}");
}

// [spec:pgorm:sem:conn.pool.config-redaction+1/test]    Serialize is untouched, because round-tripping is its job
#[test]
fn serialize_still_carries_the_password() {
    let cfg = Config {
        url: Some("postgres://john_doe:topsecret@pg.example.com/example".to_owned()),
        password: Some("topsecret".to_owned()),
        ..named()
    };

    let json = serde_json::to_string(&cfg).expect("Config serializes");

    assert!(
        json.contains("topsecret"),
        "a serializer that dropped the password would emit a config that cannot connect"
    );
}
