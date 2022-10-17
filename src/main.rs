mod config;

use crate::config::{Config, CONFIG_FILE};
use anyhow::{bail, Context};
use clap::Parser;
use log::{error, info, LevelFilter};
use simplelog::{format_description, ColorChoice, ConfigBuilder, TermLogger, TerminalMode};
use ssh2::Session;
use std::io::Read;
use std::net::TcpStream;
use std::process::exit;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {}

fn main() {
    configure_logging();

    let args = Args::parse();

    let config = match Config::load() {
        Some(config) => {
            // Save again, in case this script has additional parameters that were not yet listed
            // in the config file.
            config.save();
            config
        }
        None => {
            let default_config = Config::default();
            default_config.save();

            info!("Created {} in the current working directory. Please fill in the desired values, then run this script again.", CONFIG_FILE);

            return;
        }
    };

    if !verify_config(&config) {
        exit(1);
    }

    let result = deploy(&config);
    if let Err(error) = result {
        error!("{:#}", error);
        exit(1);
    }
}

fn verify_config(config: &Config) -> bool {
    if config.target_host.is_empty() {
        error!("Please configure a target host.");
        return false;
    }
    if config.username.is_empty() {
        error!("Please configure a username.");
        return false;
    }
    if !config.private_key_file.exists() {
        error!(
            "Private key file does not exist: `{}`",
            config.private_key_file.display()
        );
        return false;
    }
    if config.rss_r_target_test_dir.to_string_lossy().is_empty() {
        error!("Please configure a target directory for testing.");
        return false;
    }

    true
}

fn deploy(config: &Config) -> anyhow::Result<()> {
    let session = connect_and_login(config)?;

    println!("{}", execute_command(&session, "ls")?);

    Ok(())
}

fn connect_and_login(config: &Config) -> anyhow::Result<Session> {
    let target = config.host_and_port();
    info!("Connecting to `{}`", target);

    let tcp = TcpStream::connect(&target)
        .with_context(|| format!("Could not connect to `{}`", target))?;
    let mut session = Session::new()?;

    session.set_tcp_stream(tcp);
    session.handshake()?;

    let passphrase = rpassword::prompt_password(format!(
        "Please enter the passphrase for private key file `{}`:",
        config.private_key_file.display()
    ))
    .context("Could not read password")?;
    session.userauth_pubkey_file(
        &config.username,
        None,
        &config.private_key_file,
        Some(&passphrase),
    )?;

    info!("Logged in as `{}`", config.username);

    Ok(session)
}

/// Executes a given command.
/// Returns an error, and the stderr output, if the command had a non-zero exit code.
fn execute_command(session: &Session, command: &str) -> anyhow::Result<String> {
    let mut channel = session.channel_session()?;
    channel.exec(command)?;

    let mut response = String::new();
    channel.read_to_string(&mut response)?;

    let mut stderr_response = String::new();
    channel.stderr().read_to_string(&mut stderr_response)?;

    channel.wait_close()?;
    let exit_code = channel.exit_status()?;

    if exit_code == 0 {
        Ok(response)
    } else {
        bail!(
            "command `{}` failed:\n```\n{}\n```",
            command,
            stderr_response
        )
    }
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
