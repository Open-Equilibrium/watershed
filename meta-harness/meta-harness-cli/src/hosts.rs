//! The commands that link this Meta-Harness to other hosts and devices (ADR-0285, ADR-0299,
//! ADR-0300): remote access, pairing codes, and the hosts this one links to.

use meta_harness_core::{Access, DEFAULT_REMOTE_PORT, Kind, remote};

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

/// `meta invite --device | --host`: a pairing code, with the addresses to pair with.
pub async fn invite(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    let kind = match args {
        ["--device"] => Kind::Device,
        ["--host"] => Kind::Host,
        _ => return Err("meta invite needs --device or --host".to_owned()),
    };
    let status = host
        .remote_status()
        .await?
        .ok_or("Remote access is off; turn it on first with meta remote on.")?;
    let code = host.invite(kind).await?;
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
            let access = access(rest)?;
            let name = host.host_add(address, code, access).await?;
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

/// The access level that `--access` names in `args`, or the default, `auto`.
pub(crate) fn access(args: &[&str]) -> Result<Access, String> {
    match args {
        [] => Ok(Access::default()),
        ["--access", text] => Access::parse(text).ok_or_else(|| {
            format!(
                "{text:?} is no access level: read-only, approval-required, auto-accept-edits, \
                 auto or full-access."
            )
        }),
        _ => Err("meta host add takes --access LEVEL".to_owned()),
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
        assert_eq!(access(&[]), Ok(Access::Auto));
        assert_eq!(access(&["--access", "read-only"]), Ok(Access::ReadOnly));
        assert!(access(&["--access", "root"]).is_err());
    }
}
