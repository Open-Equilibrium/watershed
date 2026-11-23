use meta_harness_core::{Access, ClientKind, DEFAULT_REMOTE_PORT, remote};

use crate::{USAGE, print};

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

pub async fn invite(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    let (kind, access) = match args {
        ["--device"] => (ClientKind::Device, Access::default()),
        ["--host", rest @ ..] => (
            ClientKind::Host,
            access("meta invite", rest)?.unwrap_or_default(),
        ),
        _ => return Err("meta invite needs --device or --host".to_owned()),
    };
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
        ["--port", text] => text
            .parse()
            .map_err(|_| format!("{text:?} is not a number.")),
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
        assert!(port(&["--prot", "4000"]).is_err());
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
