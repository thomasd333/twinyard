//! One TLS stream type for suppaftp with two engines:
//! - **rustls** (default): modern ciphers only, TLS session resumption on the
//!   data channel (vsftpd's `require_ssl_reuse` needs it);
//! - **SChannel** (Windows' own TLS, via native-tls) as a compatibility mode for
//!   old servers rustls refuses — e.g. static-RSA-only key exchange
//!   (`AES256-GCM-SHA384`), which TC (OpenSSL) still accepts.
//!
//! Certificates are checked by the same `PinVerifier` in both modes.

use crate::tls::PinVerifier;
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use suppaftp::{FtpError, FtpResult, TlsConnector, TlsStream};

pub enum Inner {
    Rustls(Box<StreamOwned<ClientConnection, TcpStream>>),
    Native(Box<native_tls::TlsStream<TcpStream>>),
}

impl Read for Inner {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Inner::Rustls(s) => s.read(buf),
            Inner::Native(s) => s.read(buf),
        }
    }
}

impl Write for Inner {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Inner::Rustls(s) => s.write(buf),
            Inner::Native(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Inner::Rustls(s) => s.flush(),
            Inner::Native(s) => s.flush(),
        }
    }
}

pub struct Tls {
    inner: Inner,
    /// Send close_notify on drop (off once the socket was handed back).
    shutdown: bool,
}

impl std::fmt::Debug for Tls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.inner {
            Inner::Rustls(_) => "Tls(rustls)",
            Inner::Native(_) => "Tls(schannel)",
        })
    }
}

impl TlsStream for Tls {
    type InnerStream = Inner;

    fn tcp_stream(mut self) -> FtpResult<TcpStream> {
        let mut s = self.get_ref().try_clone().map_err(FtpError::ConnectionError)?;
        self.shutdown = false;
        let _ = s.flush();
        Ok(s)
    }

    fn get_ref(&self) -> &TcpStream {
        match &self.inner {
            Inner::Rustls(s) => s.get_ref(),
            Inner::Native(s) => s.get_ref(),
        }
    }

    fn mut_ref(&mut self) -> &mut Inner {
        &mut self.inner
    }
}

impl Drop for Tls {
    fn drop(&mut self) {
        if !self.shutdown {
            return;
        }
        match &mut self.inner {
            Inner::Rustls(s) => {
                let _ = s.flush();
                s.conn.send_close_notify();
                let _ = s.conn.write_tls(&mut s.sock);
            }
            Inner::Native(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Connector {
    Rustls(Arc<ClientConfig>),
    /// SChannel accepts any certificate in the handshake; the leaf is then
    /// checked by `verifier` (pins + Windows chain validation).
    Native { conn: native_tls::TlsConnector, verifier: Arc<PinVerifier> },
}

impl Connector {
    pub fn native(verifier: Arc<PinVerifier>) -> Result<Connector, String> {
        let conn = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Connector::Native { conn, verifier })
    }
}

impl TlsConnector for Connector {
    type Stream = Tls;

    fn connect(&self, domain: &str, tcp: TcpStream) -> FtpResult<Tls> {
        let sec = |e: String| FtpError::SecureError(e);
        match self {
            Connector::Rustls(cfg) => {
                let name = ServerName::try_from(domain.to_string()).map_err(|e| sec(e.to_string()))?;
                let conn = ClientConnection::new(cfg.clone(), name).map_err(|e| sec(e.to_string()))?;
                Ok(Tls { inner: Inner::Rustls(Box::new(StreamOwned::new(conn, tcp))), shutdown: true })
            }
            Connector::Native { conn, verifier } => {
                let s = conn.connect(domain, tcp).map_err(|e| sec(format!("SChannel: {e}")))?;
                let leaf = s
                    .peer_certificate()
                    .map_err(|e| sec(e.to_string()))?
                    .ok_or_else(|| sec(twinyard_core::i18n::tr("serwer nie przedstawił certyfikatu", "the server presented no certificate").into()))?
                    .to_der()
                    .map_err(|e| sec(e.to_string()))?;
                let name = ServerName::try_from(domain.to_string()).map_err(|e| sec(e.to_string()))?;
                verifier
                    .verify_server_cert(&CertificateDer::from(leaf), &[], &name, &[], UnixTime::now())
                    .map_err(|e| sec(format!("invalid peer certificate: {e}")))?;
                log::debug!("TLS (SChannel) data/control channel to {domain} established");
                Ok(Tls { inner: Inner::Native(Box::new(s)), shutdown: true })
            }
        }
    }
}
