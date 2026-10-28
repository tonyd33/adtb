use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;

use campaign::web::Server;

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("campaign.adt"));
    let address: SocketAddr = args
        .next()
        .map(|s| {
            s.into_string()
                .map_err(|_| "Address must be UTF-8".to_owned())
        })
        .transpose()?
        .unwrap_or_else(|| "0.0.0.0:3000".to_owned())
        .parse()
        .map_err(|e| format!("address: {e}"))?;
    if args.next().is_some() {
        return Err("Usage: campaign [DATABASE] [LISTEN_ADDRESS:PORT]".to_owned());
    }
    let server = tokio::task::spawn_blocking(move || Server::open(path))
        .await
        .unwrap_or_else(worker_failed)?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|e| format!("listen: {e}"))?;
    let address = listener
        .local_addr()
        .map_err(|e| format!("listen address: {e}"))?;
    let host = if address.ip().is_unspecified() {
        if address.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        }
    } else {
        address.ip()
    };
    writeln!(std::io::stdout(), "Listening on {address}\nGM login (one use): http://{}{}\nPlayers: use this laptop's LAN address with their seat link.", SocketAddr::new(host, address.port()), server.login_path())
        .map_err(|e| format!("print login: {e}"))?;
    if let Some(lan) = lan_address(address) {
        writeln!(
            std::io::stdout(),
            "Player address: http://{lan} (append the seat link)"
        )
        .map_err(|e| format!("print LAN address: {e}"))?;
    }
    let result = axum::serve(listener, server.router())
        .with_graceful_shutdown(async {
            if let Err(e) = tokio::signal::ctrl_c().await {
                report_signal(e);
            }
        })
        .await
        .map_err(|e| format!("server: {e}"));
    tokio::task::spawn_blocking(move || server.close())
        .await
        .unwrap_or_else(worker_failed)?;
    result
}

fn report_signal(error: std::io::Error) {
    writeln!(std::io::stderr(), "shutdown signal: {error}").unwrap_or_default();
}

fn lan_address(listen: SocketAddr) -> Option<SocketAddr> {
    if !listen.ip().is_unspecified() {
        return Some(listen);
    }
    let (bind, destination) = if listen.is_ipv4() {
        ("0.0.0.0:0", "192.0.2.1:9")
    } else {
        ("[::]:0", "[2001:db8::1]:9")
    };
    let socket = UdpSocket::bind(bind).ok()?;
    socket.connect(destination).ok()?;
    Some(SocketAddr::new(
        socket.local_addr().ok()?.ip(),
        listen.port(),
    ))
}

fn worker_failed<T>(error: tokio::task::JoinError) -> Result<T, String> {
    if error.is_panic() {
        writeln!(
            std::io::stderr(),
            "campaign database worker panicked: {error}"
        )
        .unwrap_or_default();
        std::process::abort();
    }
    Err(format!("database worker cancelled: {error}"))
}
