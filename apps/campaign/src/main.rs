use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;

use campaign::web::Server;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "campaign",
    about = "Serve the campaign database DATABASE on LISTEN_ADDRESS:PORT."
)]
struct Args {
    #[arg(
        value_name = "DATABASE",
        default_value = "campaign.adt",
        help = "the database file"
    )]
    path: PathBuf,
    #[arg(
        value_name = "LISTEN_ADDRESS:PORT",
        default_value = "0.0.0.0:3000",
        help = "the address to listen on"
    )]
    address: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let Args { path, address } = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => usage(&e),
    };
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

/// Print `e` and exit 1 for a usage error, 0 for help.
fn usage(e: &clap::Error) -> ! {
    if let Err(why) = e.print() {
        writeln!(std::io::stderr(), "print usage: {why}").unwrap_or_default();
    }
    std::process::exit(i32::from(e.use_stderr()));
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

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_is_well_formed() {
        Args::command().debug_assert();
    }

    #[test]
    fn the_readme_invocation_parses() {
        let a = Args::try_parse_from(["campaign", "campaign.adt", "0.0.0.0:3000"])
            .expect("the invocation parses");
        assert_eq!(PathBuf::from("campaign.adt"), a.path);
        assert_eq!(SocketAddr::from(([0, 0, 0, 0], 3000)), a.address);
    }

    #[test]
    fn both_arguments_have_defaults() {
        let a = Args::try_parse_from(["campaign"]).expect("no argument parses");
        assert_eq!(PathBuf::from("campaign.adt"), a.path);
        assert_eq!(SocketAddr::from(([0, 0, 0, 0], 3000)), a.address);
        let a = Args::try_parse_from(["campaign", "other.adt"]).expect("a path alone parses");
        assert_eq!(PathBuf::from("other.adt"), a.path);
    }

    #[test]
    fn a_bad_address_or_a_third_argument_is_refused() {
        assert!(Args::try_parse_from(["campaign", "c.adt", "nowhere"]).is_err());
        assert!(Args::try_parse_from(["campaign", "c.adt", "0.0.0.0:1", "x"]).is_err());
    }
}
