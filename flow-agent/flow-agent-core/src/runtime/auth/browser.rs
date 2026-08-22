use super::{MAX_OAUTH_CALLBACK_HTTP_HEAD_BYTES, protocol, transport};
use crate::runtime::{
    deadlines::AUTH_OVERALL_DEADLINE,
    oauth_credential::{CredentialRecord, auth_protocol},
    openai_codex::{OPENAI_CODEX_CLIENT_ID, OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_PROVIDER_ID},
    types::RuntimeError,
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::process::Command;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const BROWSER_REDIRECT_URL: &str = "http://localhost:1455/auth/callback";
const LOOPBACK_CALLBACK_CONNECTION_DEADLINE: Duration = Duration::from_secs(5);

pub(super) fn browser_login(
    present: &mut impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<CredentialRecord, RuntimeError> {
    let listener = TcpListener::bind(("127.0.0.1", 1455)).map_err(|_| auth_protocol())?;
    let state = protocol::random_url_token(32)?;
    let verifier = protocol::random_url_token(32)?;
    run_browser_login_with_components(
        listener,
        state,
        verifier,
        present,
        open_system_browser,
        transport::exchange_authorization_code,
    )
}

pub(crate) fn run_browser_login_with_components<P, O, E>(
    listener: TcpListener,
    state: String,
    verifier: String,
    present: &mut P,
    mut open_browser: O,
    mut exchange: E,
) -> Result<CredentialRecord, RuntimeError>
where
    P: FnMut(&str) -> Result<(), RuntimeError>,
    O: FnMut(&str) -> Result<(), RuntimeError>,
    E: FnMut(&str, &str, &str) -> Result<CredentialRecord, RuntimeError>,
{
    listener
        .set_nonblocking(true)
        .map_err(|_| auth_protocol())?;
    let challenge = protocol::base64url_encode(&Sha256::digest(verifier.as_bytes()));
    let authorize_url = build_authorize_url(&state, &challenge);
    present(&format!(
        "Open this URL to authenticate: {authorize_url}\nIf the browser callback does not complete, cancel and run flow auth login {OPENAI_CODEX_PROVIDER_ID} --device."
    ))?;
    open_browser(&authorize_url)?;
    let started = Instant::now();
    let code = loop {
        if started.elapsed() >= AUTH_OVERALL_DEADLINE {
            return Err(auth_protocol());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let result = read_loopback_callback_until(
                    &mut stream,
                    &state,
                    started + AUTH_OVERALL_DEADLINE,
                    LOOPBACK_CALLBACK_CONNECTION_DEADLINE,
                );
                let (status, body) = if result.is_ok() {
                    (
                        "200 OK",
                        "Authorization received. Return to the terminal for the final result.",
                    )
                } else {
                    ("400 Bad Request", "Authentication request rejected.")
                };
                let _ = write_loopback_response(&mut stream, status, body);
                if let Ok(code) = result {
                    break code;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return Err(auth_protocol()),
        }
    };
    exchange(&code, &verifier, BROWSER_REDIRECT_URL)
}

pub(crate) fn build_authorize_url(state: &str, challenge: &str) -> String {
    let fields = [
        ("response_type", "code"),
        ("client_id", OPENAI_CODEX_CLIENT_ID),
        ("redirect_uri", BROWSER_REDIRECT_URL),
        ("scope", "openid profile email offline_access"),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", OPENAI_CODEX_ORIGINATOR),
    ];
    let query = fields
        .into_iter()
        .map(|(name, value)| format!("{name}={}", protocol::percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{OPENAI_AUTHORIZE_URL}?{query}")
}

#[cfg(test)]
pub(crate) fn read_loopback_callback(
    stream: &mut TcpStream,
    expected_state: &str,
) -> Result<String, RuntimeError> {
    read_loopback_callback_until(
        stream,
        expected_state,
        Instant::now() + Duration::from_secs(5),
        LOOPBACK_CALLBACK_CONNECTION_DEADLINE,
    )
}

pub(crate) fn read_loopback_callback_until(
    stream: &mut TcpStream,
    expected_state: &str,
    deadline: Instant,
    connection_deadline: Duration,
) -> Result<String, RuntimeError> {
    let deadline = deadline.min(
        Instant::now()
            .checked_add(connection_deadline)
            .ok_or_else(auth_protocol)?,
    );
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let maximum = MAX_OAUTH_CALLBACK_HTTP_HEAD_BYTES;
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(auth_protocol)?;
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_secs(5))))
            .map_err(|_| auth_protocol())?;
        let read = stream.read(&mut chunk).map_err(|_| auth_protocol())?;
        if read == 0 || bytes.len().saturating_add(read) > maximum {
            return Err(auth_protocol());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let request = std::str::from_utf8(&bytes).map_err(|_| auth_protocol())?;
    let first = request.lines().next().ok_or_else(auth_protocol)?;
    let target = first
        .strip_prefix("GET ")
        .and_then(|line| line.strip_suffix(" HTTP/1.1"))
        .ok_or_else(auth_protocol)?;
    let query = target
        .strip_prefix("/auth/callback?")
        .ok_or_else(auth_protocol)?;
    protocol::parse_oauth_callback(query, expected_state)
}

pub(crate) fn write_loopback_response(
    stream: &mut TcpStream,
    status: &str,
    body: &str,
) -> Result<(), RuntimeError> {
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| auth_protocol())?;
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|_| auth_protocol())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BrowserLauncher {
    #[cfg(windows)]
    NativeWindows,
    #[cfg(unix)]
    Executable(&'static str),
}

pub(crate) fn system_browser_launcher() -> BrowserLauncher {
    #[cfg(target_os = "windows")]
    return BrowserLauncher::NativeWindows;
    #[cfg(target_os = "macos")]
    return BrowserLauncher::Executable("/usr/bin/open");
    #[cfg(all(unix, not(target_os = "macos")))]
    return BrowserLauncher::Executable("/usr/bin/xdg-open");
}

fn open_system_browser(url: &str) -> Result<(), RuntimeError> {
    match system_browser_launcher() {
        #[cfg(windows)]
        BrowserLauncher::NativeWindows => open_system_browser_with_windows_shell(url),
        #[cfg(unix)]
        BrowserLauncher::Executable(executable) => Command::new(executable)
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|_| auth_protocol()),
    }
}

#[cfg(windows)]
fn open_system_browser_with_windows_shell(url: &str) -> Result<(), RuntimeError> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let url = url
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as usize > 32 {
        Ok(())
    } else {
        Err(auth_protocol())
    }
}
