//! RPC portal launch-argument construction shared by the GUI and service
//! binaries. Kept free of easytier dependencies so the service binary does
//! not link the whole easytier stack just to build core launch args.

/// Loopback is always allowed so the app's own health checks and CLI status
/// polling keep working when the portal is exposed to the virtual network.
const LOCAL_WHITELIST: &[&str] = &["127.0.0.1/32", "::1/128"];

/// Build the `--rpc-portal` / `--rpc-portal-whitelist` args for launching
/// `easytier-core`. Shared by every core start site (GUI, kernel-update
/// recovery, service).
pub fn build_rpc_portal_args(
    remote_manage_enabled: bool,
    rpc_port: u16,
    whitelist_cidrs: &[String],
) -> Vec<String> {
    if !remote_manage_enabled {
        return vec![format!("--rpc-portal=127.0.0.1:{rpc_port}")];
    }
    let mut whitelist: Vec<String> = LOCAL_WHITELIST.iter().map(|s| s.to_string()).collect();
    for cidr in whitelist_cidrs {
        let cidr = cidr.trim();
        if !cidr.is_empty() && !whitelist.iter().any(|w| w == cidr) {
            whitelist.push(cidr.to_string());
        }
    }
    vec![
        format!("--rpc-portal=0.0.0.0:{rpc_port}"),
        format!("--rpc-portal-whitelist={}", whitelist.join(",")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_args_default_to_loopback() {
        let args = build_rpc_portal_args(false, 15888, &[]);
        assert_eq!(args, vec!["--rpc-portal=127.0.0.1:15888".to_string()]);
    }

    #[test]
    fn portal_args_remote_mode_injects_loopback_and_user_cidrs() {
        let args = build_rpc_portal_args(true, 15888, &["10.126.126.0/24".to_string()]);
        assert_eq!(args.len(), 2);
        assert!(args[0].contains("0.0.0.0:15888"));
        assert!(args[1].starts_with("--rpc-portal-whitelist="));
        let list = args[1].trim_start_matches("--rpc-portal-whitelist=");
        assert!(list.contains("127.0.0.1/32"));
        assert!(list.contains("::1/128"));
        assert!(list.contains("10.126.126.0/24"));
    }

    #[test]
    fn portal_args_remote_mode_dedups_and_skips_blank() {
        let args = build_rpc_portal_args(
            true,
            15888,
            &["  ".to_string(), "127.0.0.1/32".to_string(), "10.0.0.0/8".to_string()],
        );
        let list = args[1].trim_start_matches("--rpc-portal-whitelist=");
        assert_eq!(list.matches("127.0.0.1/32").count(), 1);
        assert!(list.contains("10.0.0.0/8"));
    }
}
