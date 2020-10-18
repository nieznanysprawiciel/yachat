use actix::{Actor, System};
use structopt::{clap, StructOpt};

use chat::Chat;

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

fn main() -> Result<(), anyhow::Error> {
    dotenv::dotenv().ok();
    flexi_logger::Logger::with_env()
        .log_to_file()
        .directory("logs")
        .start()
        .expect("Failed to initialize logging");

    let args = Args::from_args();
    log::info!("Starting ya-chat.");

    let sys = System::new("ya-chat");

    Chat::new(args)?.start();

    match sys.run() {
        Err(e) => {
            log::error!("Finished with error: {}", e);
            std::process::exit(1)
        }
        Ok(_) => std::process::exit(0),
    }
}
