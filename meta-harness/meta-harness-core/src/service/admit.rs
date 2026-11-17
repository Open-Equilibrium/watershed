//! The handshake of a connection (ADR-0299): which listener it came in on, what its token admits
//! and when its open connections end.

use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request};
use tokio_tungstenite::tungstenite::http::StatusCode;

use super::Shared;
use crate::api::{self, Call, HostsCall};
use crate::config::now_ms;
use crate::hosts::{self, ClientKind};

/// Which listener a connection came in on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Listener {
    /// This computer's loopback, which admits the local token only.
    Local,
    /// The remote addresses, which admit paired clients only (ADR-0299).
    Remote,
}

/// Who a session serves (ADR-0299): this computer's own Meta-Harness, or a client that paired.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Caller {
    Local,
    Device,
    Host,
}

impl Caller {
    /// Whether this caller may make `call`. A device makes the calls that reach its own computer's
    /// projects, threads, Git and configuration, but not the ones only this computer makes; a host
    /// asks only for a T3 pairing code (ADR-0288).
    pub(super) fn answers(self, call: &Call) -> bool {
        match self {
            Self::Local => true,
            Self::Device => matches!(
                call,
                Call::Host(_)
                    | Call::Project { .. }
                    | Call::Thread { .. }
                    | Call::Git { .. }
                    | Call::Config(_)
            ),
            Self::Host => matches!(call, Call::Hosts(HostsCall::T3Pairing)),
        }
    }

    /// Whether this caller may open streams; a host opens none.
    pub(super) fn streams(self) -> bool {
        self != Self::Host
    }
}

/// What a handshake admits a connection to.
pub(super) enum Admission {
    /// Calls and streams, as this caller, for the client named if it is one.
    Session(Caller, Option<String>),
    /// A host's tunnel to its T3 server (ADR-0288), for the host named.
    Tunnel(String),
    /// A new client, which receives its token once before the connection closes.
    Paired(String),
}

pub(super) fn refuse(status: StatusCode, reason: impl Into<String>) -> ErrorResponse {
    let mut refused = ErrorResponse::new(Some(reason.into()));
    *refused.status_mut() = status;
    refused
}

pub(super) fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.headers().get(name).and_then(|v| v.to_str().ok())
}

/// Refuses a request that names no API version, or another, since it would misread the frames.
// tungstenite's handshake callback returns its own large error response.
#[allow(clippy::result_large_err)]
fn check_version(request: &Request) -> Result<(), ErrorResponse> {
    let version = header(request, api::API_HEADER).and_then(|v| v.parse::<u32>().ok());
    if version == Some(api::API_VERSION) {
        return Ok(());
    }
    let mut refused = refuse(
        StatusCode::UPGRADE_REQUIRED,
        format!("Meta-Harness speaks API version {}.", api::API_VERSION),
    );
    refused
        .headers_mut()
        .insert(api::API_HEADER, api::API_VERSION.into());
    Err(refused)
}

/// Decides what a handshake admits (ADR-0299). The local listener admits the local token. The
/// remote listener admits a paired client's token, or a pairing code, which pays with a new token.
// tungstenite's handshake callback returns its own large error response.
#[allow(clippy::result_large_err)]
pub(super) fn admit(
    shared: &Shared,
    listener: Listener,
    request: &Request,
) -> Result<Admission, ErrorResponse> {
    let bearer = header(request, "authorization").and_then(|v| v.strip_prefix("Bearer "));
    match listener {
        Listener::Local => {
            if !bearer.is_some_and(|b| hosts::same(b.as_bytes(), shared.token.as_bytes())) {
                return Err(refuse(
                    StatusCode::UNAUTHORIZED,
                    "Meta-Harness needs its token.",
                ));
            }
            check_version(request)?;
            Ok(Admission::Session(Caller::Local, None))
        }
        Listener::Remote => {
            check_version(request)?;
            let db = shared.store.db();
            if let Some(code) = header(request, api::PAIRING_HEADER) {
                let name = header(request, api::CLIENT_HEADER).unwrap_or("unnamed");
                let token = hosts::pair(&db, code, name, now_ms())
                    .map_err(|reason| refuse(StatusCode::FORBIDDEN, reason))?;
                return Ok(Admission::Paired(token));
            }
            let client = match bearer {
                Some(token) => hosts::client(&db, token)
                    .map_err(|error| refuse(StatusCode::INTERNAL_SERVER_ERROR, error))?,
                None => None,
            };
            let tunnel = header(request, api::TUNNEL_HEADER).is_some();
            match client {
                Some((ClientKind::Device, _)) if tunnel => Err(refuse(
                    StatusCode::FORBIDDEN,
                    "Only another host may tunnel to this computer's T3 Code.",
                )),
                Some((ClientKind::Host, name)) if tunnel => Ok(Admission::Tunnel(name)),
                Some((ClientKind::Device, name)) => {
                    Ok(Admission::Session(Caller::Device, Some(name)))
                }
                Some((ClientKind::Host, name)) => Ok(Admission::Session(Caller::Host, Some(name))),
                None => Err(refuse(
                    StatusCode::UNAUTHORIZED,
                    "Meta-Harness needs a token of a paired client.",
                )),
            }
        }
    }
}

/// Resolves when the client named `owner` is revoked, and never for a connection with no client.
/// A connection that fell more than [`REVOKED`] revocations behind ends, so that it cannot miss
/// one; a client still paired comes back with its token.
pub(super) async fn revoked(
    events: &mut tokio::sync::broadcast::Receiver<String>,
    owner: Option<&str>,
) {
    let Some(owner) = owner else {
        return std::future::pending::<()>().await;
    };
    loop {
        match events.recv().await {
            Ok(name) if name == owner => return,
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => return,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return std::future::pending::<()>().await;
            }
        }
    }
}
