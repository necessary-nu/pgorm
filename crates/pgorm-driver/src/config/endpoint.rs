#[cfg(feature = "runtime")]
use super::{ChannelBinding, LoadBalanceHosts, SslMode, SslNegotiation, TargetSessionAttrs};
use super::{Config, Error, Path, PathBuf};

/// Refuse a read-only `:memory:` database, which would be an empty one
/// nothing could ever write, before the engine is asked to open it.
// [spec:pgorm:req:read-only.turso]
pub(crate) fn refuse_read_only_memory(path: &Path) -> Result<(), Error> {
    if path
        .to_str()
        .is_some_and(|path| path.starts_with(":memory:"))
    {
        return Err(Error::turso_unsupported("a read-only `:memory:` database"));
    }
    Ok(())
}

pub(super) struct NativeConfig {
    pub(super) path: PathBuf,
    pub(super) read_only: bool,
    #[cfg(feature = "turso")]
    pub(super) database: tokio::sync::OnceCell<crate::TursoDatabase>,
}

impl NativeConfig {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            read_only: false,
            #[cfg(feature = "turso")]
            database: tokio::sync::OnceCell::new(),
        }
    }
}

impl Clone for NativeConfig {
    // [spec:pgorm:req:turso.pool+1] cloning settings cannot share a live in-memory database
    fn clone(&self) -> Self {
        // Cloned configuration describes a new owner, not a shared memory database.
        Self {
            read_only: self.read_only,
            ..Self::new(self.path.clone())
        }
    }
}

impl PartialEq for NativeConfig {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.read_only == other.read_only
    }
}
impl Eq for NativeConfig {}

impl Config {
    /// Whether this configuration selects an embedded Turso database.
    ///
    /// Parsing preserves this identity even when the `turso` feature is disabled.
    // [spec:pgorm:req:turso.pool+1]
    pub fn is_turso(&self) -> bool {
        self.turso.is_some()
    }

    /// The embedded database path, or `None` for a PostgreSQL connection.
    pub fn get_turso_path(&self) -> Option<&Path> {
        self.turso.as_ref().map(|native| native.path.as_path())
    }

    /// Open the embedded database read-only, so the engine refuses every
    /// write with SQLSTATE `25006`.
    ///
    /// Refused at once for `:memory:`, which read-only would be an empty
    /// database nothing could write, and for a PostgreSQL configuration,
    /// which has no file to open.
    // [spec:pgorm:req:read-only.turso]
    pub fn turso_read_only(&mut self) -> Result<&mut Config, Error> {
        let Some(native) = self.turso.as_mut() else {
            return Err(Error::turso_unsupported(
                "opening a PostgreSQL connection as a read-only embedded database",
            ));
        };
        refuse_read_only_memory(&native.path)?;
        native.read_only = true;
        Ok(self)
    }

    /// Whether the embedded database opens read-only.
    pub fn get_turso_read_only(&self) -> bool {
        self.turso.as_ref().is_some_and(|native| native.read_only)
    }

    #[cfg(feature = "runtime")]
    pub(super) fn validate_native(&self) -> Result<(), Error> {
        let defaults = Config::new();
        let configured = self.user.is_some()
            || self.password.is_some()
            || self.dbname.is_some()
            || self.options.is_some()
            || self.application_name.is_some()
            || !self.host.is_empty()
            || !self.hostaddr.is_empty()
            || !self.port.is_empty()
            || self.connect_timeout.is_some()
            || self.tcp_user_timeout.is_some()
            || self.ssl_mode != SslMode::Prefer
            || self.ssl_negotiation != SslNegotiation::Postgres
            || !self.keepalives
            || self.target_session_attrs != TargetSessionAttrs::Any
            || self.channel_binding != ChannelBinding::Prefer
            || self.load_balance_hosts != LoadBalanceHosts::Disable;
        #[cfg(not(target_arch = "wasm32"))]
        let configured = configured || self.keepalive_config != defaults.keepalive_config;
        let _ = defaults;
        if configured {
            return Err(Error::turso_unsupported(
                "PostgreSQL connection options on an embedded database",
            ));
        }
        Ok(())
    }
}

// Explicit paths disambiguate filenames containing '=' or reserved connection keywords.
pub(super) fn native_path(input: &str) -> Result<Option<PathBuf>, Error> {
    let bytes = input.as_bytes();
    let windows = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let explicit = input.starts_with('/')
        || input.starts_with("./")
        || input.starts_with("../")
        || input.starts_with('\\')
        || windows;
    let path_before_assignment = input
        .find('/')
        .is_some_and(|slash| input.find('=').is_none_or(|equals| slash < equals));
    if input != ":memory:" && !explicit {
        let word = input.split_whitespace().next().unwrap_or("");
        let reserved = matches!(
            word,
            "host"
                | "hostaddr"
                | "port"
                | "user"
                | "password"
                | "dbname"
                | "options"
                | "application_name"
                | "sslmode"
                | "sslnegotiation"
                | "connect_timeout"
                | "tcp_user_timeout"
                | "keepalives"
                | "keepalives_idle"
                | "keepalives_interval"
                | "keepalives_retries"
                | "target_session_attrs"
                | "channel_binding"
                | "load_balance_hosts"
        );
        if input.trim().is_empty()
            || input.contains(':')
            || reserved
            || (input.contains('=') && !path_before_assignment)
        {
            return Ok(None);
        }
    }
    if input.contains('\0') {
        return Err(Error::config_parse(
            "embedded database paths cannot contain NUL".into(),
        ));
    }
    Ok(Some(PathBuf::from(input)))
}
