//! The connector through its public API, under each crypto provider.
#![cfg(any(feature = "aws-lc-rs", feature = "ring"))]

use std::sync::Arc;

use pgorm_driver::tls::MakeTlsConnect;
use pgorm_driver_rustls::MakeRustlsConnect;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    Error, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
};
use tokio::io::DuplexStream;

fn client_config_with_provider(provider: rustls::crypto::CryptoProvider) -> rustls::ClientConfig {
    rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .expect("default protocol versions")
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth()
}

#[cfg(feature = "aws-lc-rs")]
fn aws_lc_rs_client_config() -> rustls::ClientConfig {
    client_config_with_provider(rustls::crypto::aws_lc_rs::default_provider())
}

#[cfg(feature = "ring")]
fn ring_client_config() -> rustls::ClientConfig {
    client_config_with_provider(rustls::crypto::ring::default_provider())
}

#[derive(Debug)]
struct AcceptAllVerifier {}

impl ServerCertVerifier for AcceptAllVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::ED25519,
        ]
    }
}

async fn connect_works(mut config: rustls::ClientConfig) {
    env_logger::builder().is_test(true).try_init().unwrap();

    config
        .dangerous()
        .set_certificate_verifier(Arc::new(AcceptAllVerifier {}));
    let tls = MakeRustlsConnect::new(config);
    let (client, conn) = pgorm_driver::connect(
        "sslmode=require host=localhost port=5432 user=postgres",
        tls,
    )
    .await
    .expect("connect");
    tokio::spawn(async move { conn.await.map_err(|e| panic!("{e:?}")) });
    let stmt = client.prepare("SELECT 1").await.expect("prepare");
    let _ = client.query(&stmt, &[]).await.expect("query");
}

#[cfg(feature = "aws-lc-rs")]
#[tokio::test]
async fn it_works_with_aws_lc_rs() {
    connect_works(aws_lc_rs_client_config()).await;
}

#[cfg(feature = "ring")]
#[tokio::test]
async fn it_works_with_ring() {
    connect_works(ring_client_config()).await;
}

fn accepts_socket_host_before_tls(config: rustls::ClientConfig) {
    let mut tls = MakeRustlsConnect::new(config);

    let tls_connect = <MakeRustlsConnect as MakeTlsConnect<DuplexStream>>::make_tls_connect(
        &mut tls,
        "/var/run/postgresql",
    );

    assert!(tls_connect.is_ok());
}

#[cfg(feature = "aws-lc-rs")]
#[test]
fn socket_host_accepted_with_aws_lc_rs() {
    accepts_socket_host_before_tls(aws_lc_rs_client_config());
}

#[cfg(feature = "ring")]
#[test]
fn socket_host_accepted_with_ring() {
    accepts_socket_host_before_tls(ring_client_config());
}
