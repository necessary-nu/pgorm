//! A binding's TLS options, read into the connector pgorm connects through.
//!
//! A binding's pool verifies the server's certificate chain and host name
//! unless it is told otherwise in so many words: the connection string's
//! `sslmode=disable`, or a `tls` mode that verifies less. The string's own
//! `prefer` and `require` keep full verification, so a weaker mode is always
//! the program's explicit choice and never a default a DSN carried in.
// [spec:pgorm:req:bindings.tls]

use std::sync::Arc;

use pgorm::{Config, SslMode};
use rustls::{
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        verify_server_cert_signed_by_trust_anchor,
    },
    crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject},
    server::ParsedCertificate,
};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::{Dialect, Refusal};

/// How a connection uses TLS, named as libpq's `sslmode` names it.
///
/// libpq's `allow`, which tries plaintext first and turns to TLS only when
/// the server refuses it, has no counterpart: tokio-postgres negotiates TLS
/// before anything else, so there is nothing to fall back from, and an
/// attacker answering in the server's place always gets the plaintext.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// Plaintext only.
    Disable,
    /// TLS when the server offers it, plaintext when it does not, with no
    /// certificate verified.
    Prefer,
    /// TLS always, with no certificate verified.
    Require,
    /// TLS always, the server's certificate verified to chain to a
    /// certificate the program trusts, whatever host it names.
    VerifyCa,
    /// TLS always, the server's certificate verified to chain to a trusted
    /// certificate and to name the host connected to.
    VerifyFull,
}

impl TlsMode {
    /// Every mode with its spelling, which is libpq's.
    pub const NAMES: [(&'static str, Self); 5] = [
        ("disable", Self::Disable),
        ("prefer", Self::Prefer),
        ("require", Self::Require),
        ("verify-ca", Self::VerifyCa),
        ("verify-full", Self::VerifyFull),
    ];

    /// The mode spelled `name`.
    pub fn named(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(spelling, _)| *spelling == name)
            .map(|(_, mode)| *mode)
    }

    /// The mode's spelling.
    pub fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, mode)| *mode == self)
            .map_or("", |(spelling, _)| spelling)
    }

    fn verifies(self) -> bool {
        matches!(self, Self::VerifyCa | Self::VerifyFull)
    }
}

/// The TLS a binding's options ask for: a mode, the certificates a
/// verifying mode trusts, and a client certificate with its private key, each
/// as PEM.
#[derive(Debug, Clone, Default)]
pub struct TlsSettings {
    /// The `tls` option; absent, the connection string's `sslmode` decides.
    pub mode: Option<TlsMode>,
    /// The `ca` option: PEM certificates trusted in place of the platform's
    /// store.
    pub ca: Option<Vec<u8>>,
    /// The `cert` option: the PEM certificate chain the client presents.
    pub cert: Option<Vec<u8>>,
    /// The `key` option: the PEM private key of `cert`.
    pub key: Option<Vec<u8>>,
}

/// What a connection connects through.
#[derive(Clone)]
pub enum Connector {
    /// Plaintext: pgorm's `NoTls`.
    Plain,
    /// TLS through rustls, configured as the settings asked.
    Rustls(MakeRustlsConnect),
}

impl std::fmt::Debug for Connector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Plain => "Connector::Plain",
            Self::Rustls(_) => "Connector::Rustls",
        })
    }
}

impl TlsSettings {
    /// The connector these settings ask for, `config`'s `sslmode` set to
    /// match: `disable` for plaintext, `prefer` for a connection that may be
    /// either, and `require` otherwise, so that no verifying connection falls
    /// back to plaintext.
    ///
    /// A mode that contradicts the string, a CA a mode would not use, a
    /// client certificate without its key, PEM that holds nothing usable, and
    /// a platform store with no root to verify against are each refused here,
    /// before anything is sent.
    // [spec:pgorm:req:bindings.tls]
    pub fn connector(&self, config: &mut Config, dialect: Dialect) -> Result<Connector, Refusal> {
        let tls = |mode: TlsMode| dialect.setting("tls", mode.name());
        let mode = match self.mode {
            Some(mode) => mode,
            None if config.get_ssl_mode() == SslMode::Disable => TlsMode::Disable,
            None => TlsMode::VerifyFull,
        };
        let demanded = config.get_ssl_mode() == SslMode::Require;
        if mode == TlsMode::Disable {
            if self.ca.is_some() || self.cert.is_some() || self.key.is_some() || demanded {
                return Err(Refusal(format!(
                    "TLS configuration conflicts with {}",
                    tls(mode)
                )));
            }
            config.ssl_mode(SslMode::Disable);
            return Ok(Connector::Plain);
        }
        if mode == TlsMode::Prefer && demanded {
            return Err(Refusal(format!(
                "sslmode=require conflicts with {}, which allows plaintext",
                tls(mode)
            )));
        }
        if self.ca.is_some() && !mode.verifies() {
            return Err(Refusal(format!(
                "ca is trusted only to verify the server, which {} does not do",
                tls(mode)
            )));
        }
        let verifier = match mode {
            TlsMode::VerifyFull => Verify::Full(roots(self.ca.as_deref(), dialect)?),
            TlsMode::VerifyCa => match self.ca.as_deref() {
                Some(ca) => Verify::Chain(roots(Some(ca), dialect)?),
                None => {
                    return Err(Refusal(format!(
                        "{} needs {}: the certificates the server's certificate must chain to",
                        tls(mode),
                        dialect.ca()
                    )));
                }
            },
            _ => Verify::Nothing,
        };
        let client = client_certificate(self.cert.as_deref(), self.key.as_deref())?;
        config.ssl_mode(if mode == TlsMode::Prefer {
            SslMode::Prefer
        } else {
            SslMode::Require
        });
        Ok(Connector::Rustls(MakeRustlsConnect::new(client_config(
            verifier, client,
        )?)))
    }
}

/// What a TLS connection checks of the server's certificate.
enum Verify {
    /// Its chain to `roots` and the host name: rustls' own verifier.
    Full(RootCertStore),
    /// Its chain to `roots` alone.
    Chain(RootCertStore),
    /// Nothing but the handshake's signatures, which prove the server holds
    /// the key of the certificate it sent.
    Nothing,
}

type ClientCertificate = (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>);

fn client_config(
    verify: Verify,
    client: Option<ClientCertificate>,
) -> Result<ClientConfig, Refusal> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let algorithms = provider.signature_verification_algorithms;
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| Refusal("TLS protocol initialization failed".to_owned()))?;
    let builder = match verify {
        Verify::Full(roots) => builder.with_root_certificates(roots),
        Verify::Chain(roots) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(ChainOnly {
                roots: Arc::new(roots),
                algorithms,
            })),
        Verify::Nothing => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SignaturesOnly { algorithms })),
    };
    match client {
        None => Ok(builder.with_no_client_auth()),
        Some((chain, key)) => builder.with_client_auth_cert(chain, key).map_err(|error| {
            Refusal(format!(
                "cert and key are not a client certificate rustls can present: {error}"
            ))
        }),
    }
}

/// The certificates `ca` holds, or the platform's trusted roots.
fn roots(ca: Option<&[u8]>, dialect: Dialect) -> Result<RootCertStore, Refusal> {
    let mut roots = RootCertStore::empty();
    match ca {
        Some(pem) => {
            for certificate in certificates(pem, "ca")? {
                roots
                    .add(certificate)
                    .map_err(|_| Refusal("ca holds a certificate rustls cannot use".to_owned()))?;
            }
        }
        None => platform_roots(&mut roots, dialect)?,
    }
    Ok(roots)
}

/// Every certificate in the PEM `option` gave, at least one.
fn certificates(pem: &[u8], option: &str) -> Result<Vec<CertificateDer<'static>>, Refusal> {
    let certificates = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Refusal(format!("{option} holds invalid PEM certificate data")))?;
    if certificates.is_empty() {
        return Err(Refusal(format!("{option} holds no certificate")));
    }
    Ok(certificates)
}

/// The client certificate and its key, both or neither.
fn client_certificate(
    cert: Option<&[u8]>,
    key: Option<&[u8]>,
) -> Result<Option<ClientCertificate>, Refusal> {
    match (cert, key) {
        (None, None) => Ok(None),
        (Some(cert), Some(key)) => {
            let chain = certificates(cert, "cert")?;
            let key = PrivateKeyDer::from_pem_slice(key).map_err(|_| {
                Refusal(
                    "key holds no private key rustls can use: an unencrypted PKCS #8, \
                     PKCS #1 or SEC1 key in PEM"
                        .to_owned(),
                )
            })?;
            Ok(Some((chain, key)))
        }
        _ => Err(Refusal(
            "cert and key go together: give the client certificate and its private key, or neither"
                .to_owned(),
        )),
    }
}

/// Adds the platform's trusted roots to `roots`: the system keychain's on
/// macOS, the system store's on Windows, and elsewhere the CA bundle and
/// directory OpenSSL would use. `SSL_CERT_FILE` and `SSL_CERT_DIR`, when set,
/// replace the platform's store entirely.
///
/// A store that yields no usable root fails here, at construction, naming
/// what the loader reported; it never leaves a pool that trusts nothing and
/// so fails every connection later with an unexplained certificate error.
/// The loader also reports conditions that are routine on a working host,
/// such as macOS having no per-user trust settings, so its errors are only
/// surfaced when no root was found.
fn platform_roots(roots: &mut RootCertStore, dialect: Dialect) -> Result<(), Refusal> {
    let found = rustls_native_certs::load_native_certs();
    let (added, _) = roots.add_parsable_certificates(found.certs);
    if added > 0 {
        return Ok(());
    }
    let reported = found
        .errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let detail = if reported.is_empty() {
        String::new()
    } else {
        format!(" ({})", reported.join("; "))
    };
    let replaced = std::env::var_os("SSL_CERT_FILE").is_some()
        || std::env::var_os("SSL_CERT_DIR").is_some_and(|dirs| {
            std::env::split_paths(&dirs).any(|dir| !dir.as_os_str().is_empty())
        });
    let ca = dialect.ca();
    Err(Refusal(if replaced {
        format!(
            "SSL_CERT_FILE and SSL_CERT_DIR, which replace the platform's certificate store, \
             name no usable root certificate{detail}; pass {ca}, or point them at a CA bundle"
        )
    } else {
        format!(
            "the platform's certificate store holds no usable root certificate{detail}; \
             pass {ca}, or point SSL_CERT_FILE or SSL_CERT_DIR at a CA bundle"
        )
    }))
}

/// `verify-ca`: the server's certificate chains to a trusted root, for
/// server authentication, whatever host name it carries. The chain is
/// checked exactly as rustls' own verifier checks it, short of the name.
#[derive(Debug)]
struct ChainOnly {
    roots: Arc<RootCertStore>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let certificate = ParsedCertificate::try_from(end_entity)?;
        verify_server_cert_signed_by_trust_anchor(
            &certificate,
            &self.roots,
            intermediates,
            now,
            self.algorithms.all,
        )?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// `prefer` and `require`: any certificate is accepted, and only the
/// handshake's signatures are checked, so the session is encrypted to
/// whoever holds the key of the certificate the server sent. That is
/// libpq's meaning of the two modes, and the program's explicit choice.
#[derive(Debug)]
struct SignaturesOnly {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for SignaturesOnly {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dsn: &str) -> Config {
        dsn.parse().expect("the test's connection string parses")
    }

    fn refused(settings: &TlsSettings, dsn: &str) -> String {
        match settings.connector(&mut config(dsn), Dialect::Python) {
            Err(refusal) => refusal.message().to_owned(),
            Ok(_) => panic!("{settings:?} over {dsn} was not refused"),
        }
    }

    const PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\nMA==\n-----END CERTIFICATE-----\n";

    #[test]
    fn every_mode_reads_back_its_spelling() {
        for (name, mode) in TlsMode::NAMES {
            assert_eq!(TlsMode::named(name), Some(mode));
            assert_eq!(mode.name(), name);
        }
        assert_eq!(TlsMode::named("allow"), None);
    }

    #[test]
    fn plaintext_refuses_certificates_and_demanded_tls() {
        let disable = TlsSettings {
            mode: Some(TlsMode::Disable),
            ..TlsSettings::default()
        };
        let mut plain = config("host=db.example.com");
        assert!(matches!(
            disable.connector(&mut plain, Dialect::Python),
            Ok(Connector::Plain)
        ));
        assert_eq!(plain.get_ssl_mode(), SslMode::Disable);
        for settings in [
            TlsSettings {
                ca: Some(PEM.to_vec()),
                ..disable.clone()
            },
            TlsSettings {
                cert: Some(PEM.to_vec()),
                ..disable.clone()
            },
        ] {
            assert_eq!(
                refused(&settings, "host=db.example.com"),
                "TLS configuration conflicts with tls='disable'"
            );
        }
        assert_eq!(
            refused(&disable, "host=db.example.com sslmode=require"),
            "TLS configuration conflicts with tls='disable'"
        );
    }

    #[test]
    fn weaker_modes_refuse_what_they_would_ignore() {
        let prefer = TlsSettings {
            mode: Some(TlsMode::Prefer),
            ..TlsSettings::default()
        };
        assert_eq!(
            refused(&prefer, "host=db.example.com sslmode=require"),
            "sslmode=require conflicts with tls='prefer', which allows plaintext"
        );
        let require = TlsSettings {
            mode: Some(TlsMode::Require),
            ca: Some(PEM.to_vec()),
            ..TlsSettings::default()
        };
        assert_eq!(
            refused(&require, "host=db.example.com"),
            "ca is trusted only to verify the server, which tls='require' does not do"
        );
        let chain = TlsSettings {
            mode: Some(TlsMode::VerifyCa),
            ..TlsSettings::default()
        };
        assert_eq!(
            refused(&chain, "host=db.example.com"),
            "tls='verify-ca' needs ca or cafile: the certificates the server's certificate must chain to"
        );
    }

    #[test]
    fn a_client_certificate_comes_with_its_key() {
        let half = TlsSettings {
            mode: Some(TlsMode::Require),
            cert: Some(PEM.to_vec()),
            ..TlsSettings::default()
        };
        assert_eq!(
            refused(&half, "host=db.example.com"),
            "cert and key go together: give the client certificate and its private key, or neither"
        );
        let unkeyed = TlsSettings {
            key: Some(b"not a key".to_vec()),
            ..half
        };
        assert!(refused(&unkeyed, "host=db.example.com").starts_with("key holds no private key"));
    }

    #[test]
    fn pem_holding_no_certificate_is_refused() {
        let empty = TlsSettings {
            ca: Some(b"not a certificate".to_vec()),
            ..TlsSettings::default()
        };
        assert_eq!(
            refused(&empty, "host=db.example.com"),
            "ca holds no certificate"
        );
    }

    #[test]
    fn unverified_modes_set_the_strings_sslmode() {
        for (mode, expected) in [
            (TlsMode::Prefer, SslMode::Prefer),
            (TlsMode::Require, SslMode::Require),
        ] {
            let mut target = config("host=db.example.com");
            let settings = TlsSettings {
                mode: Some(mode),
                ..TlsSettings::default()
            };
            assert!(matches!(
                settings.connector(&mut target, Dialect::JavaScript),
                Ok(Connector::Rustls(_))
            ));
            assert_eq!(target.get_ssl_mode(), expected);
        }
    }
}
