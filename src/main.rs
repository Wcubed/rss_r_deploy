mod config;

use crate::config::{Config, CONFIG_FILE};
use clap::Parser;
use log::{info, LevelFilter};
use simplelog::{format_description, ColorChoice, ConfigBuilder, TermLogger, TerminalMode};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {}

fn main() {
    configure_logging();

    let args = Args::parse();

    let config = match Config::load() {
        Some(config) => config,
        None => {
            let default_config = Config::default();
            default_config.save();

            info!("Created {} in the current working directory. Please fill in the desired values, then run this script again.", CONFIG_FILE);

            return;
        }
    };
}

fn configure_logging() {
    // The logged time is by default in UTC.
    let config = ConfigBuilder::default()
        .set_time_format_custom(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second]"
        ))
        .set_thread_level(LevelFilter::Trace)
        .set_target_level(LevelFilter::Trace)
        .build();

    TermLogger::init(
        // TODO (Wybe 2022-07-16): Allow changing this through command line arguments
        LevelFilter::Info,
        config,
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .expect("Could not start logger.");
}
