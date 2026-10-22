use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};

use mixum_env_core::container::PortMapping;

use crate::output::{self, CliError, Result};

pub const PORT_START: u16 = 17600;
pub const PORT_END: u16 = 17999;
pub const MAP_PORT: u16 = 7601;

pub fn port_is_free(port: u16) -> Result<bool> {
    if port == 0 {
        return Ok(false);
    }
    match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)) {
        Ok(listener) => {
            drop(listener);
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Ok(false),
        Err(error) => Err(CliError::io(format!(
            "cannot probe 127.0.0.1:{port}: {error}"
        ))),
    }
}

pub fn used_host_ports(containers: &[Vec<PortMapping>]) -> Vec<u16> {
    let mut ports = containers
        .iter()
        .flatten()
        .map(|mapping| mapping.host)
        .collect::<Vec<_>>();
    ports.sort_unstable();
    ports.dedup();
    ports
}

pub fn allocate(requested: u16, used: &[u16]) -> Result<u16> {
    if requested != 0 {
        if used.contains(&requested) {
            return Err(CliError::conflict(format!(
                "port {requested} is already published by a mixum environment"
            )));
        }
        if !port_is_free(requested)? {
            return Err(CliError::conflict(format!(
                "port {requested} is already in use on 127.0.0.1"
            )));
        }
        return Ok(requested);
    }
    for port in PORT_START..=PORT_END {
        if used.contains(&port) {
            continue;
        }
        if port_is_free(port)? {
            return Ok(port);
        }
    }
    Err(CliError::runtime(format!(
        "no free MAP port in {PORT_START}-{PORT_END}; stop an environment or pass --port"
    )))
}

pub async fn check_command(port: u16, json: bool) -> Result<()> {
    let free = port_is_free(port)?;
    if json {
        output::print_json(&serde_json::json!({ "port": port, "free": free }));
    } else if free {
        println!("port {port} is free on 127.0.0.1");
    } else {
        return Err(CliError::conflict(format!(
            "port {port} is not available on 127.0.0.1"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_used_ports() {
        assert!(allocate(17601, &[17601]).is_err());
    }

    #[test]
    fn allocates_in_range() {
        let port = allocate(0, &[]).unwrap();
        assert!((PORT_START..=PORT_END).contains(&port));
    }
}
