use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request};
use tokio_tungstenite::tungstenite::http::StatusCode;

use super::Shared;
use crate::api::{self, Call, HostsCall};
use crate::config::now_ms;
use crate::hosts::{self, Access, ClientKind};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Listener {
    Local,
    Remote,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Caller {
    Local,
    Device,
    Host(Access),
}

impl Caller {
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
            Self::Host(_) => matches!(call, Call::Hosts(HostsCall::T3Pairing)),
        }
    }

    pub(super) fn streams(self) -> bool {
        !matches!(self, Self::Host(_))
    }

    pub(super) fn access(self) -> Access {
        match self {
            Self::Host(access) => access,
            Self::Local | Self::Device => Access::default(),
        }
    }
}

pub(super) enum Admission {
    Session(Caller, Option<String>),
    Tunnel(String, Access),
    Paired(String, Access),
}

pub(super) fn refuse(status: StatusCode, reason: impl Into<String>) -> ErrorResponse {
    let mut refused = ErrorResponse::new(Some(reason.into()));
    *refused.status_mut() = status;
    refused
}

pub(super) fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.headers().get(name).and_then(|v| v.to_str().ok())
}

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
                let (token, access) = hosts::pair(&db, code, name, now_ms())
                    .map_err(|reason| refuse(StatusCode::FORBIDDEN, reason))?;
                return Ok(Admission::Paired(token, access));
            }
            let client = match bearer {
                Some(token) => hosts::client(&db, token)
                    .map_err(|error| refuse(StatusCode::INTERNAL_SERVER_ERROR, error))?,
                None => None,
            };
            let tunnel = header(request, api::TUNNEL_HEADER).is_some();
            match client {
                Some((ClientKind::Device, _, _)) if tunnel => Err(refuse(
                    StatusCode::FORBIDDEN,
                    "Only another host may tunnel to this computer's T3 Code.",
                )),
                Some((ClientKind::Host, access, name)) if tunnel => {
                    Ok(Admission::Tunnel(name, access))
                }
                Some((ClientKind::Device, _, name)) => {
                    Ok(Admission::Session(Caller::Device, Some(name)))
                }
                Some((ClientKind::Host, access, name)) => {
                    Ok(Admission::Session(Caller::Host(access), Some(name)))
                }
                None => Err(refuse(
                    StatusCode::UNAUTHORIZED,
                    "Meta-Harness needs a token of a paired client.",
                )),
            }
        }
    }
}

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
