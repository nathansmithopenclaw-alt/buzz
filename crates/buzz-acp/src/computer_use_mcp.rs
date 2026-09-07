//! Provider-neutral computer-use registration supplied by the execution host.
use crate::acp::{EnvVar, McpServer};

pub(crate) fn from_environment() -> Option<McpServer> {
    from_values(|key| std::env::var(key).ok())
}

fn from_values(value: impl Fn(&str) -> Option<String>) -> Option<McpServer> {
    let node = value("BUZZ_COMPUTER_NODE")?;
    let script = value("BUZZ_COMPUTER_SCRIPT")?;
    let registry = value("BUZZ_COMPUTER_REGISTRY")?;
    let owner = value("BUZZ_COMPUTER_OWNER")?;
    if [&node, &script, &registry]
        .iter()
        .any(|p| !std::path::Path::new(p).is_absolute())
        || owner.trim().is_empty()
    {
        tracing::warn!("computer-use registration rejected: invalid host-owned configuration");
        return None;
    }
    let mut server = McpServer {
        name: "buzz-computer-use".into(),
        command: node,
        args: vec![script],
        env: vec![
            EnvVar {
                name: "BUZZ_COMPUTER_REGISTRY".into(),
                value: registry,
            },
            EnvVar {
                name: "BUZZ_COMPUTER_OWNER".into(),
                value: owner,
            },
        ],
    };
    if let (Some(driver), Some(socket)) = (value("BUZZ_NATIVE_DRIVER"), value("BUZZ_NATIVE_SOCKET"))
    {
        if [&driver, &socket]
            .iter()
            .all(|p| std::path::Path::new(p).is_absolute())
        {
            server.env.push(EnvVar {
                name: "BUZZ_NATIVE_DRIVER".into(),
                value: driver,
            });
            server.env.push(EnvVar {
                name: "BUZZ_NATIVE_SOCKET".into(),
                value: socket,
            });
        }
    }
    if let Some(label) = value("BUZZ_COMPUTER_LABEL") {
        server.env.push(EnvVar {
            name: "BUZZ_COMPUTER_LABEL".into(),
            value: label,
        });
    }
    Some(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_complete_absolute_host_config_and_never_forwards_signing_keys() {
        let values = std::collections::HashMap::from([
            ("BUZZ_COMPUTER_NODE", "/runtime/node"),
            ("BUZZ_COMPUTER_SCRIPT", "/runtime/main.mjs"),
            ("BUZZ_COMPUTER_REGISTRY", "/private/sessions"),
            ("BUZZ_COMPUTER_OWNER", "community/agent"),
            ("BUZZ_PRIVATE_KEY", "must-not-forward"),
        ]);
        let server =
            from_values(|k| values.get(k).map(|s| s.to_string())).expect("complete config");
        assert_eq!(server.env.len(), 2);
        assert!(server
            .env
            .iter()
            .all(|e| !e.value.contains("must-not-forward")));
        assert!(from_values(|k| (k != "BUZZ_COMPUTER_OWNER")
            .then(|| values.get(k).map(|s| s.to_string()))
            .flatten())
        .is_none());
        assert!(from_values(|k| Some(if k == "BUZZ_COMPUTER_SCRIPT" {
            "relative.js".into()
        } else {
            values.get(k)?.to_string()
        }))
        .is_none());
    }
}
