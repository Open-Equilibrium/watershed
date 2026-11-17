//! The commands that link this Meta-Harness to other hosts and devices (ADR-0285, ADR-0299,
//! ADR-0300): remote access, pairing codes, and the hosts this one links to.

use meta_harness_core::{Access, ClientKind, DEFAULT_REMOTE_PORT, remote};

/// `meta remote on [--port N] | off | status`.
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
                Some(status) => println!("Remote access is on at {}.", status.addresses.join(", ")),
                None => println!("Remote access is off."),
            }
            Ok(())
        }
        _ => Err("unknown remote command; see meta help".to_owned()),
    }
}

/// `meta invite --device | --host [--access LEVEL]`: a pairing code, with the addresses to pair
/// with. A host's code gives its link the level, `auto` by default (ADR-0300).
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
    let code = host.invite(kind, access).await?;
    println!("Pairing code {code}, valid for 5 minutes and for one use.");
    for address in &status.addresses {
        println!("  address {address}");
    }
    Ok(())
}

/// `meta host add|list|remove|revoke|agents`.
pub async fn host(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    match args {
        ["add", address, code, rest @ ..] => {
            let (name, access) = host
                .host_add(address, code, access("meta host add", rest)?)
                .await?;
            println!("Linked {name} at {} access.", access.as_str());
            Ok(())
        }
        ["list"] => {
            let listed = host.hosts().await?;
            let text = serde_json::to_string_pretty(&listed).map_err(|e| e.to_string())?;
            println!("{text}");
            Ok(())
        }
        ["remove", name] => {
            host.host_remove(name).await?;
            println!("Removed {name}.");
            Ok(())
        }
        ["revoke", name] => {
            host.revoke_client(name).await?;
            println!("Revoked {name}.");
            Ok(())
        }
        ["agents", "on"] => host.host_agents(true).await,
        ["agents", "off"] => host.host_agents(false).await,
        _ => Err("unknown host command; see meta help".to_owned()),
    }
}

/// The port that `--port` names in `args`, or the default.
pub(crate) fn port(args: &[&str]) -> Result<u16, String> {
    match args {
        [] => Ok(DEFAULT_REMOTE_PORT),
        ["--port", text] => text
            .parse()
            .map_err(|_| format!("{text:?} is no port number.")),
        _ => Err("meta remote on takes --port N".to_owned()),
    }
}

/// The access level that `--access` names in `args`, if it names one; `command` names the command
/// in the error.
pub(crate) fn access(command: &str, args: &[&str]) -> Result<Option<Access>, String> {
    match args {
        [] => Ok(None),
        ["--access", text] => Access::parse(text).map(Some).ok_or_else(|| {
            format!(
                "{text:?} is no access level: read-only, approval-required, auto-accept-edits, \
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
        assert!(port(&["--port", "many"]).is_err());
        assert!(port(&["--prot", "4000"]).is_err());
        assert_eq!(access("meta host add", &[]), Ok(None));
        assert_eq!(
            access("meta host add", &["--access", "read-only"]),
            Ok(Some(Access::ReadOnly))
        );
        assert!(access("meta host add", &["--access", "root"]).is_err());
    }
}
