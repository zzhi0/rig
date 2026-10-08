use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};

use clap::{Parser, Subcommand};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the local API and built Web UI on IPv4 loopback.
    Serve {
        /// Local TCP port. Use 0 to let the operating system select a port.
        #[arg(long, default_value_t = 7878)]
        port: u16,
        /// Built Web UI directory, relative to the current working directory.
        #[arg(long, default_value = "web/dist")]
        web_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { port, web_dir } => {
            let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
            println!("Rig is listening at http://{}", listener.local_addr()?);

            let server = axum::serve(listener, rig::http::router(web_dir))
                .with_graceful_shutdown(shutdown_signal());
            server.await
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for Ctrl-C");
}
