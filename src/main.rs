mod config;

use crate::config::{Config, CONFIG_FILE};
use anyhow::{bail, Context};
use clap::Parser;
use log::{error, info, LevelFilter};
use simplelog::{format_description, ColorChoice, ConfigBuilder, TermLogger, TerminalMode};
use ssh2::Session;
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::exit;

const REMOTE_TEMP_DIR: &str = "/tmp";

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

    let result = deploy_to_test_dir_and_run(&config);
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
    if !config.rss_r_zip.exists() {
        error!(
            "rss_r package zip does not exist: `{}`",
            config.rss_r_zip.display()
        );
        return false;
    }
    if config.rss_r_target_test_dir.to_string_lossy().is_empty() {
        error!("Please configure a target directory for testing.");
        return false;
    }
    if !config.rss_r_test_config_file.exists() {
        error!(
            "test config file does not exist: `{}`",
            config.rss_r_test_config_file.display()
        );
        return false;
    }

    true
}

fn deploy_to_test_dir_and_run(config: &Config) -> anyhow::Result<()> {
    let session = connect_and_login(config)?;

    info!("Uploading rss_r to the test directory.");
    let package_name = config
        .rss_r_zip
        .file_name()
        .context("Cannot upload file, path does not have file name.")?;
    let mut remote_temp_path = PathBuf::from(REMOTE_TEMP_DIR);
    remote_temp_path.push(package_name);

    upload_file(&session, &config.rss_r_zip, &remote_temp_path)?;

    info!(
        "Unpacking package to `{}`",
        config.rss_r_target_test_dir.display()
    );
    execute_command(
        &session,
        &format!("rm -rf '{}'", config.rss_r_target_test_dir.display()),
    )?;
    execute_command(
        &session,
        &format!(
            "unzip '{}' -d '{}'",
            remote_temp_path.display(),
            config.rss_r_target_test_dir.display()
        ),
    )?;

    info!("Transferring app config file.");
    let mut config_file_target = PathBuf::from(&config.rss_r_target_test_dir);
    config_file_target.push("rss_r");
    config_file_target.push("persistence");

    execute_command(
        &session,
        &format!("mkdir -p '{}'", config_file_target.display()),
    )?;

    config_file_target.push("app_config.ron");

    upload_file(
        &session,
        &config.rss_r_test_config_file,
        &config_file_target,
    )?;

    let mut exec_path = PathBuf::from(&config.rss_r_target_test_dir);
    // Top directory in the .zip should be rss_r.
    exec_path.push("rss_r");
    // Executable is also called rss_r.
    exec_path.push("rss_r");

    let mut working_dir = PathBuf::from(&config.rss_r_target_test_dir);
    working_dir.push("rss_r");

    info!("Running `{}`", exec_path.display());

    // Make sure to have the working directory be the same as the rss_r directory,
    // so that the program can locate the persistence and config files properly.
    execute_command(
        &session,
        &format!("cd '{}'; '{}'", working_dir.display(), exec_path.display()),
    )?;

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

fn upload_file(session: &Session, file: &Path, remote_path: &Path) -> anyhow::Result<()> {
    let mut local_file = File::open(file)?;
    let mut bytes = Vec::new();
    local_file.read_to_end(&mut bytes)?;

    info!(
        "Uploading `{}` to `{}`",
        file.display(),
        remote_path.display()
    );

    let mut remote_file = session.scp_send(&remote_path, 0o644, bytes.len() as u64, None)?;

    remote_file.write_all(&bytes)?;
    remote_file.send_eof()?;
    remote_file.wait_eof()?;
    remote_file.close()?;
    remote_file.wait_close()?;

    Ok(())
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
