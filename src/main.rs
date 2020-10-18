use actix::Actor;
use structopt::{clap, StructOpt};
use tokio::signal;

use chat::Chat;
use discover::Shutdown;

use ya_client::cli::ApiOpts;

mod chat;
mod discover;
mod protocol;

#[derive(structopt::StructOpt)]
#[structopt(global_setting = clap::AppSettings::ColoredHelp)]
pub struct Args {
    #[structopt(long, short)]
    pub name: String,
    #[structopt(long, short)]
    pub group: String,
    #[structopt(flatten)]
    pub api: ApiOpts,
}

#[actix_rt::main]
async fn main() -> Result<(), anyhow::Error> {
    dotenv::dotenv().ok();
    flexi_logger::Logger::with_env()
        .log_to_file()
        .directory("logs")
        .start()
        .expect("Failed to initialize logging");

    let args = Args::from_args();
    log::info!("Starting ya-chat.");

    let chat = Chat::new(args)?.start();

    signal::ctrl_c().await.unwrap();

    println!("Shutting down. Wait for cleanup...");
    chat.send(Shutdown {}).await??;
    println!("Finished.");
    Ok(())
}
