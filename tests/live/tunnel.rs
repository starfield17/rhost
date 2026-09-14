use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use super::support::{Live, number, shell_quote, text};

fn free_local_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|error| format!("reserve local port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("read local port: {error}"))
}

#[test]
fn socks_tunnel_persists_across_cli_processes_and_closes_cleanly() -> Result<(), String> {
    let live = Live::new("tunnel-persist")?;
    let port = free_local_port()?;
    let listen = format!("127.0.0.1:{port}");
    let opened = live.ok(&[
        "--json",
        "tunnel",
        "open",
        live.host(),
        "--kind",
        "socks",
        "--listen",
        &listen,
    ])?;
    let id = text(&opened.value, "/data/tunnel_id")?.to_string();
    let result: Result<(), String> = (|| {
        let listed = live.ok(&["--json", "tunnel", "list"])?;
        let rows = listed.value["data"]["tunnels"]
            .as_array()
            .ok_or("tunnel.list did not return an array")?;
        assert!(
            rows.iter()
                .any(|row| row["tunnel_id"] == id && row["status"] == "alive")
        );
        TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}")
                .parse()
                .map_err(|error| format!("parse listen address: {error}"))?,
            Duration::from_secs(3),
        )
        .map_err(|error| format!("SOCKS listener is not reachable: {error}"))?;
        Ok(())
    })();
    let closed = live.ok(&["--json", "tunnel", "close", &id]);
    result?;
    closed?;
    assert!(TcpStream::connect(format!("127.0.0.1:{port}")).is_err());
    Ok(())
}

#[test]
fn reverse_tunnel_carries_bytes_from_remote_to_local() -> Result<(), String> {
    let live = Live::new("tunnel-reverse")?;
    let server = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("start local service: {error}"))?;
    server
        .set_nonblocking(false)
        .map_err(|error| format!("configure local service: {error}"))?;
    let destination = server
        .local_addr()
        .map_err(|error| format!("local service address: {error}"))?
        .to_string();
    let port_answer = live.exec(
        "python3 -c \"import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()\"",
    )?;
    let port = text(&port_answer.value, "/data/output/stdout/content")?
        .trim()
        .to_string();
    let listen = format!("127.0.0.1:{port}");
    let opened = live.ok(&[
        "--json",
        "tunnel",
        "open",
        live.host(),
        "--kind",
        "reverse",
        "--listen",
        &listen,
        "--destination",
        &destination,
    ])?;
    let id = text(&opened.value, "/data/tunnel_id")?.to_string();
    let remote_program = format!(
        "python3 -c {}",
        shell_quote(&format!(
            "import socket; s=socket.create_connection(('127.0.0.1',{port}),5); s.sendall(b'rhost-reverse'); s.close()"
        ))
    );
    let child = live
        .command(&["--json", "exec", live.host(), "--command", &remote_program])
        .spawn()
        .map_err(|error| format!("start reverse client: {error}"))?;
    let mut connection = server
        .accept()
        .map_err(|error| format!("accept reverse traffic: {error}"))?
        .0;
    connection
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("set reverse read timeout: {error}"))?;
    let mut bytes = Vec::new();
    connection
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read reverse traffic: {error}"))?;
    assert_eq!(bytes, b"rhost-reverse");
    let output = super::support::wait_bounded(child, Duration::from_secs(30))?;
    let answer = super::support::decode(output, &["reverse client"])?;
    assert_eq!(number(&answer.value, "/data/execution/exit_code")?, 0);
    live.ok(&["--json", "tunnel", "close", &id])?;
    Ok(())
}
