use std::io::{Read, Write};
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
        assert!(rows.iter().any(|row| row["tunnel_id"] == id
            && row["master_alive"] == true
            && row["destination_evidence"] == "unknown"));
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

#[test]
fn live_master_keeps_destination_unknown_when_a_connect_fails() -> Result<(), String> {
    let live = Live::new("tunnel-destination")?;
    let port_answer = live.exec(
        "python3 -c \"import socket; s=socket.socket(); s.bind(('localhost',0)); print(s.getsockname()[1]); s.close()\"",
    )?;
    let remote_port = text(&port_answer.value, "/data/output/stdout/content")?.trim();
    let loopback = std::net::Ipv4Addr::LOCALHOST;
    let listen = format!("{loopback}:{}", free_local_port()?);
    let destination = format!("{loopback}:{remote_port}");
    let opened = live.ok(&[
        "tunnel",
        "open",
        live.host(),
        "--kind",
        "local",
        "--listen",
        &listen,
        "--destination",
        &destination,
        "--json",
    ])?;
    let id = text(&opened.value, "/data/tunnel_id")?;
    let observed = (|| {
        assert_eq!(opened.value["data"]["master_alive"], true);
        assert_eq!(opened.value["data"]["destination_evidence"], "unknown");
        let mut client = TcpStream::connect(&listen).map_err(|e| e.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let mut byte = [0];
        match client.read(&mut byte) {
            Ok(0) => {}
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            other => {
                return Err(format!(
                    "expected the forward connection to close, got {other:?}"
                ));
            }
        }
        let listed = live.ok(&["tunnel", "list", "--json"])?;
        let row = listed.value["data"]["tunnels"]
            .as_array()
            .ok_or("tunnel records")?
            .iter()
            .find(|row| row["tunnel_id"] == id)
            .ok_or("tunnel record missing")?;
        assert_eq!(row["master_alive"], true);
        assert_eq!(row["destination_evidence"], "unknown");
        Ok(())
    })();
    let closed = live.ok(&["tunnel", "close", id, "--json"]);
    observed?;
    closed?;
    Ok(())
}

fn start_remote_echo(live: &Live) -> Result<u16, String> {
    let py = [
        "import os,socket",
        "s=socket.socket();s.bind(('127.0.0.1',0));s.listen(2);s.settimeout(15)",
        "p=s.getsockname()[1];pid=os.fork()",
        "if pid==0:",
        " fd=os.open('/dev/null',os.O_RDWR)",
        " [os.dup2(fd,i) for i in (0,1,2)]",
        " if fd>2: os.close(fd)",
        " try:",
        "  c,_=s.accept();c.settimeout(5);d=c.recv(64);c.sendall(d);c.close()",
        " finally: s.close()",
        " os._exit(0)",
        "s.close();print(p)",
    ]
    .join("\n");
    let answer = live.exec(&format!("python3 -c {}", shell_quote(&py)))?;
    assert_eq!(number(&answer.value, "/data/execution/exit_code")?, 0);
    text(&answer.value, "/data/output/stdout/content")?
        .trim()
        .parse()
        .map_err(|e| format!("parse remote echo port: {e}"))
}

#[test]
fn live_helper_local_tunnel_reports_reachable_and_refused_evidence() -> Result<(), String> {
    let live = Live::new("tunnel-helper-local")?;
    let echo_port = start_remote_echo(&live)?;
    let local_open = format!("127.0.0.1:{}", free_local_port()?);
    let dest_open = format!("127.0.0.1:{echo_port}");
    let opened = live.ok(&[
        "tunnel",
        "open",
        live.host(),
        "--kind",
        "local",
        "--listen",
        &local_open,
        "--destination",
        &dest_open,
        "--mode",
        "helper",
        "--json",
    ])?;
    let open_id = text(&opened.value, "/data/tunnel_id")?.to_string();
    let check_open: Result<(), String> = (|| {
        assert_eq!(opened.value["data"]["master_alive"], true);
        assert_eq!(opened.value["data"]["destination_evidence"], "unknown");
        let mut client = TcpStream::connect(&local_open).map_err(|e| e.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        client
            .write_all(b"rhost-helper-local")
            .map_err(|e| e.to_string())?;
        let mut echoed = [0u8; 18];
        client.read_exact(&mut echoed).map_err(|e| e.to_string())?;
        assert_eq!(&echoed, b"rhost-helper-local");

        let listed = live.ok(&["tunnel", "list", "--json"])?;
        let row = listed.value["data"]["tunnels"]
            .as_array()
            .ok_or("tunnel records")?
            .iter()
            .find(|row| row["tunnel_id"] == open_id)
            .ok_or("open helper tunnel missing")?;
        assert_eq!(row["master_alive"], true);
        assert_eq!(row["destination_evidence"], "reachable");
        Ok(())
    })();
    let closed_open = live.ok(&["tunnel", "close", &open_id, "--json"]);
    check_open?;
    closed_open?;

    let port_answer = live.exec(
        "python3 -c \"import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()\"",
    )?;
    let closed_remote_port = text(&port_answer.value, "/data/output/stdout/content")?.trim();
    let local_closed = format!("127.0.0.1:{}", free_local_port()?);
    let dest_closed = format!("127.0.0.1:{closed_remote_port}");
    let refused_open = live.ok(&[
        "tunnel",
        "open",
        live.host(),
        "--kind",
        "local",
        "--listen",
        &local_closed,
        "--destination",
        &dest_closed,
        "--mode",
        "helper",
        "--json",
    ])?;
    let refused_id = text(&refused_open.value, "/data/tunnel_id")?.to_string();
    let check_refused: Result<(), String> = (|| {
        let mut client = TcpStream::connect(&local_closed).map_err(|e| e.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let mut byte = [0];
        let _ = client.read(&mut byte);
        let listed = live.ok(&["tunnel", "list", "--json"])?;
        let row = listed.value["data"]["tunnels"]
            .as_array()
            .ok_or("tunnel records")?
            .iter()
            .find(|row| row["tunnel_id"] == refused_id)
            .ok_or("refused helper tunnel missing")?;
        assert_eq!(row["master_alive"], true);
        assert_eq!(row["destination_evidence"], "refused");
        Ok(())
    })();
    let closed_refused = live.ok(&["tunnel", "close", &refused_id, "--json"]);
    check_refused?;
    closed_refused?;
    Ok(())
}

#[test]
fn live_helper_socks5_tunnel_persists_and_relays_traffic() -> Result<(), String> {
    let live = Live::new("tunnel-helper-socks")?;
    let echo_port = start_remote_echo(&live)?;
    let socks_listen = format!("127.0.0.1:{}", free_local_port()?);
    let opened = live.ok(&[
        "tunnel",
        "open",
        live.host(),
        "--kind",
        "socks",
        "--listen",
        &socks_listen,
        "--mode",
        "helper",
        "--json",
    ])?;
    let id = text(&opened.value, "/data/tunnel_id")?.to_string();
    let result: Result<(), String> = (|| {
        let mut client = TcpStream::connect(&socks_listen).map_err(|e| e.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        client
            .write_all(&[0x05, 0x01, 0x00])
            .map_err(|e| e.to_string())?;
        let mut auth = [0u8; 2];
        client.read_exact(&mut auth).map_err(|e| e.to_string())?;
        assert_eq!(auth, [0x05, 0x00]);

        let mut req = vec![0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1];
        req.extend(echo_port.to_be_bytes());
        client.write_all(&req).map_err(|e| e.to_string())?;
        let mut rep = [0u8; 10];
        client.read_exact(&mut rep).map_err(|e| e.to_string())?;
        assert_eq!(rep[..2], [0x05, 0x00]);

        client
            .write_all(b"rhost-helper-socks")
            .map_err(|e| e.to_string())?;
        let mut echoed = [0u8; 18];
        client.read_exact(&mut echoed).map_err(|e| e.to_string())?;
        assert_eq!(&echoed, b"rhost-helper-socks");

        let listed = live.ok(&["tunnel", "list", "--json"])?;
        let row = listed.value["data"]["tunnels"]
            .as_array()
            .ok_or("tunnel records")?
            .iter()
            .find(|row| row["tunnel_id"] == id)
            .ok_or("socks helper tunnel missing")?;
        assert_eq!(row["master_alive"], true);
        assert_eq!(row["destination_evidence"], "reachable");
        Ok(())
    })();
    let closed = live.ok(&["tunnel", "close", &id, "--json"]);
    result?;
    closed?;
    assert!(TcpStream::connect(&socks_listen).is_err());
    Ok(())
}
