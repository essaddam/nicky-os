//! Standalone archive process; it has no access to the command runtime.
use clap::{Parser, Subcommand};
use moli_i18n::tr;
use std::{net::SocketAddr, path::PathBuf, process::ExitCode};
#[derive(Debug, Parser)]
#[command(name = "moli-archive", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    NewEpoch {
        #[arg(long, env = "MOLI_ARCHIVE_DATA", default_value = "data/archive")]
        data: PathBuf,
        #[arg(long, env = "MOLI_ARCHIVE_INSTALLATION")]
        installation: String,
    },
    Serve {
        #[arg(long, env = "MOLI_ARCHIVE_DATA", default_value = "data/archive")]
        data: PathBuf,
        #[arg(long, env = "MOLI_ARCHIVE_INSTALLATION")]
        installation: String,
        #[arg(long, env = "MOLI_ARCHIVE_LISTEN", default_value = "127.0.0.1:8791")]
        listen: SocketAddr,
        #[arg(long, env = "MOLI_ARCHIVE_TOKEN_FILE")]
        token_file: PathBuf,
        #[arg(long, env = "MOLI_ARCHIVE_LANGUAGE", default_value = "fr")]
        language: String,
    },
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::NewEpoch { data, installation } => {
            moli_archive::Archive::open(&data, &installation)
                .and_then(|mut archive| archive.rotate_epoch())
                .map(|status| {
                    println!(
                        "{}",
                        serde_json::to_string(&status).expect("serializable status")
                    );
                })
        }
        Command::Serve {
            data,
            installation,
            listen,
            token_file,
            language,
        } => {
            if !moli_i18n::set_language(&language) {
                eprintln!("{}", tr!("archive.invalid_request"));
                return ExitCode::FAILURE;
            }
            match moli_archive::Archive::open(&data, &installation) {
                Ok(archive) => {
                    moli_archive::http::serve(archive, installation, listen, &token_file).await
                }
                Err(e) => Err(e),
            }
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: {e:#}", tr!("archive.storage_error"));
            ExitCode::FAILURE
        }
    }
}
