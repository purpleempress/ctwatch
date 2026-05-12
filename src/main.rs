use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "ctwatch", version, about = "Self-hosted CT mirror")]
struct Cli {
    #[command(subcommand)]
    command: ctwatch::cmd::Command,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    ctwatch::cmd::dispatch(cli.command).await
}
