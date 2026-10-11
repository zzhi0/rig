use std::{
    future::Future,
    io,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use rig::{
    chatgpt::{ChatGpt, available_models},
    chatgpt_auth::ChatGptAuth,
    runtime::{RunEvent, RunOutcome},
    tools::ReadFiles,
};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Continue with ChatGPT to authorize use of your plan.
    Login,
    /// Revoke the ChatGPT session and remove local tokens.
    Logout,
    /// List models available to the signed-in ChatGPT account.
    Models,
    /// Run one agent task using your ChatGPT plan.
    Run {
        /// Task to delegate to the agent.
        #[arg(value_parser = clap::builder::NonEmptyStringValueParser::new())]
        prompt: String,
        /// Model ID from `rig models`.
        #[arg(long)]
        model: String,
        /// Allow the agent to read this file. Repeat for additional files.
        #[arg(long = "read", value_name = "FILE")]
        files: Vec<PathBuf>,
    },
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
async fn main() -> ExitCode {
    match execute(Cli::parse()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(cli: Cli) -> io::Result<ExitCode> {
    match cli.command {
        Command::Login => {
            let auth = ChatGptAuth::new().map_err(io::Error::other)?;
            account_command(async {
                let identity = auth.login().await?;
                println!("Signed in with ChatGPT: {identity}");
                Ok(())
            })
            .await
        }
        Command::Logout => {
            let auth = ChatGptAuth::new().map_err(io::Error::other)?;
            account_command(async {
                auth.logout().await?;
                println!("Signed out of ChatGPT.");
                Ok(())
            })
            .await
        }
        Command::Models => {
            let auth = ChatGptAuth::new().map_err(io::Error::other)?;
            account_command(async {
                for model in available_models(&auth).await? {
                    println!("{model}");
                }
                Ok(())
            })
            .await
        }
        Command::Run {
            prompt,
            model,
            files,
        } => run_agent(prompt, model, files).await,
        Command::Serve { port, web_dir } => {
            let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
            println!("Rig is listening at http://{}", listener.local_addr()?);

            let server = axum::serve(listener, rig::http::router(web_dir))
                .with_graceful_shutdown(shutdown_signal());
            server.await?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn run_agent(prompt: String, model: String, files: Vec<PathBuf>) -> io::Result<ExitCode> {
    let tools = ReadFiles::new(files).map_err(io::Error::other)?;
    let auth = ChatGptAuth::new().map_err(io::Error::other)?;
    let model = ChatGpt::new(auth, model).map_err(io::Error::other)?;
    let report = rig::runtime::run(
        &model,
        &tools,
        prompt,
        shutdown_signal(),
        |event| match event {
            RunEvent::ModelStarted => eprintln!("Calling ChatGPT..."),
            RunEvent::ToolStarted(call) => eprintln!("Tool: {}", call.name),
            RunEvent::ToolFinished(result) if result.is_error => {
                eprintln!("Tool failed: {}", result.content);
            }
            _ => {}
        },
    )
    .await;

    match report.outcome {
        RunOutcome::Completed(output) => {
            println!("{output}");
            Ok(ExitCode::SUCCESS)
        }
        RunOutcome::Failed(error) => Err(io::Error::other(error)),
        RunOutcome::Cancelled => {
            eprintln!("Cancelled.");
            Ok(ExitCode::from(130))
        }
    }
}

async fn account_command(
    operation: impl Future<Output = Result<(), String>>,
) -> io::Result<ExitCode> {
    tokio::select! {
        biased;
        () = shutdown_signal() => {
            eprintln!("Cancelled.");
            Ok(ExitCode::from(130))
        }
        result = operation => {
            result.map_err(io::Error::other)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for Ctrl-C");
}
