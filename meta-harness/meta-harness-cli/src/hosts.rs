use std::num::IntErrorKind;

use meta_harness_core::{Access, ClientKind, DEFAULT_REMOTE_PORT, remote};

use crate::{USAGE, print};

const PORT_RANGE: &str = "The port must be 1-65535.";

pub async fn remote(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    match args {
        ["on", rest @ ..] => {
            let status = host.remote_on(port(rest)?).await?;
            println!("Remote access is on at {}.", status.addresses.join(", "));
            Ok(())
        }
        ["off"] => {
            host.remote_off().await?;
            println!("Remote access is off.");
            Ok(())
        }
        ["status"] => {
            match host.remote_status().await? {
                Some(status) => match status.waiting {
                    Some(reason) => println!("Remote access is on, not listening yet: {reason}"),
                    None => println!("Remote access is on at {}.", status.addresses.join(", ")),
                },
                None => println!("Remote access is off."),
            }
            Ok(())
        }
        _ => Err(format!("unknown remote command\n{USAGE}")),
    }
}

fn invite_request(args: &[&str]) -> Result<(ClientKind, Access), String> {
    match args {
        ["--device"] => Ok((ClientKind::Device, Access::default())),
        ["--host", rest @ ..] => Ok((
            ClientKind::Host,
            access("meta invite", rest)?.unwrap_or_default(),
        )),
        ["--device", "--access", ..] => Err("--access goes only with --host.".to_owned()),
        _ => Err("meta invite needs --device or --host".to_owned()),
    }
}

pub async fn invite(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    let (kind, access) = invite_request(args)?;
    let status = host
        .remote_status()
        .await?
        .ok_or("Remote access is off; turn it on first with meta remote on.")?;
    if let Some(reason) = status.waiting {
        return Err(format!("Remote access is on, not listening yet: {reason}"));
    }
    let code = host.invite(kind, access).await?;
    println!("Pairing code {code}, valid for 5 minutes and for one use.");
    for address in &status.addresses {
        println!("  address {address}");
    }
    Ok(())
}

pub async fn host(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    match args {
        ["add", address, code, rest @ ..] => {
            let (name, access) = host
                .host_add(address, code, access("meta host add", rest)?)
                .await?;
            println!("Linked {name} at {} access.", access.as_str());
            Ok(())
        }
        ["list"] => print(&host.hosts().await?),
        ["remove", name] => {
            let client = host.host_remove(name).await?;
            println!(
                "Removed {name}. Its T3 session stays until \"meta host revoke {client}\" runs on {name}."
            );
            Ok(())
        }
        ["revoke", name] => {
            host.revoke_client(name).await?;
            println!("Revoked {name}.");
            Ok(())
        }
        ["agents", "on"] => {
            host.host_agents(true).await?;
            println!("Agents from other hosts are on here.");
            Ok(())
        }
        ["agents", "off"] => {
            host.host_agents(false).await?;
            println!("Agents from other hosts are off here.");
            Ok(())
        }
        _ => Err(format!("unknown host command\n{USAGE}")),
    }
}

pub(crate) fn port(args: &[&str]) -> Result<u16, String> {
    match args {
        [] => Ok(DEFAULT_REMOTE_PORT),
        ["--port", text] => match text.parse::<u16>() {
            Ok(0) => Err(PORT_RANGE.to_owned()),
            Ok(port) => Ok(port),
            Err(error) if matches!(error.kind(), IntErrorKind::PosOverflow) => {
                Err(PORT_RANGE.to_owned())
            }
            Err(_) => Err(format!("{text:?} is not a number.")),
        },
        _ => Err("meta remote on takes --port N".to_owned()),
    }
}

pub(crate) fn access(command: &str, args: &[&str]) -> Result<Option<Access>, String> {
    match args {
        [] => Ok(None),
        ["--access", text] => Access::parse(text).map(Some).ok_or_else(|| {
            format!(
                "{text:?} is not an access level: read-only, approval-required, auto-accept-edits, \
                 auto or full-access."
            )
        }),
        _ => Err(format!("{command} takes --access LEVEL")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_take_their_defaults_and_refuse_what_they_do_not_name() {
        assert_eq!(port(&[]), Ok(3774));
        assert_eq!(port(&["--port", "4000"]), Ok(4000));
        assert_eq!(
            port(&["--port", "many"]),
            Err("\"many\" is not a number.".to_owned())
        );
        for out_of_range in ["0", "65536", "99999", "99999999999999999999"] {
            assert_eq!(
                port(&["--port", out_of_range]),
                Err("The port must be 1-65535.".to_owned()),
                "{out_of_range}"
            );
        }
        assert!(port(&["--prot", "4000"]).is_err());
        assert_eq!(
            invite_request(&["--device", "--access", "auto"]).err(),
            Some("--access goes only with --host.".to_owned())
        );
        assert_eq!(
            invite_request(&["--bogus"]).err(),
            Some("meta invite needs --device or --host".to_owned())
        );
        assert_eq!(access("meta host add", &[]), Ok(None));
        assert_eq!(
            access("meta host add", &["--access", "read-only"]),
            Ok(Some(Access::ReadOnly))
        );
        assert_eq!(
            access("meta host add", &["--access", "root"]),
            Err("\"root\" is not an access level: read-only, approval-required, auto-accept-edits, auto or full-access.".to_owned())
        );
    }
}
