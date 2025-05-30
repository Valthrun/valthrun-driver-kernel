use alloc::string::String;
use core::convert::Infallible;

use embedded_io::WriteFmtError;
use thiserror::Error;
use vtk_wsk::WskError;

#[derive(Error, Debug)]
pub enum HttpError {
    #[error("wsk has not been initialized")]
    NotInitialized,

    #[error("dns lookup failed: {0}")]
    DnsLookupFailure(WskError),

    #[error("dns lookup yielded no results")]
    DnsNoResults,

    #[error("unexpected EOF")]
    EOF,

    #[error("io: {0}")]
    WskTransportError(#[from] WskError),

    #[error("tls: {0:?}")]
    TlsTransportError(embedded_tls::TlsError),

    #[error("connect failed: {0}")]
    ConnectError(anyhow::Error),

    #[error("response headers too long")]
    ResponseHeadersTooLong,

    #[error("uncomplete response headers")]
    ResponseHeadersUncomplete,

    #[error("invalid response header {0}")]
    ResponseHeaderInvalid(String),

    #[error("response headers invalid: {0}")]
    ResponseHeadersInvalid(httparse::Error),

    #[error("fmt error")]
    WriteFmtError(WriteFmtError<Infallible>),

    #[error("host header in request not set")]
    MissingHostHeader,
}

impl From<embedded_tls::TlsError> for HttpError {
    fn from(value: embedded_tls::TlsError) -> Self {
        HttpError::TlsTransportError(value)
    }
}

impl From<WriteFmtError<Infallible>> for HttpError {
    fn from(value: WriteFmtError<Infallible>) -> Self {
        HttpError::WriteFmtError(value)
    }
}
