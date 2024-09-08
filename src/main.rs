mod config;

use crate::config::{Config, CONFIG_FILE};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser;
use color_eyre::eyre::{eyre, OptionExt};
use color_eyre::{eyre::WrapErr, Result};
use log::{error, info, LevelFilter};
use signal_hook::consts::SIGINT;
use signal_hook::iterator::Signals;
use simplelog::{format_description, ColorChoice, ConfigBuilder, TermLogger, TerminalMode};
use ssh2::{ExtendedData, Session};
use std::fs::File;
use std::io::{stdout, Read, Write};
use std::net::TcpStream;
use std::process::exit;

const REMOTE_TEMP_DIR: &str = "/tmp";

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(short, long)]
    production: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    configure_logging()?;

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

            exit(1);
        }
    };

    if !verify_config(&config) {
        exit(1);
    }

    if args.production {
        deploy_production(&config)?;
    } else {
        deploy_to_test_dir(&config)?;
    }

    Ok(())
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
    if !config.rss_r_zip.exists() {
        error!("rss_r package zip does not exist: `{}`", config.rss_r_zip);
        return false;
    }
    if config.rss_r_target_test_dir.to_string().is_empty() {
        error!("Please configure a target directory for testing.");
        return false;
    }
    if !config.rss_r_test_config_file.exists() {
        error!(
            "test config file does not exist: `{}`",
            config.rss_r_test_config_file
        );
        return false;
    }

    if config.rss_r_production_directory.to_string().is_empty() {
        error!("Please configure a target directory for production.");
        return false;
    }
    if config.rss_r_production_user.is_empty() {
        error!("Please configure a production user.");
        return false;
    }

    true
}

fn deploy_production(config: &Config) -> Result<()> {
    let session = connect_and_login(config)?;

    info!("Stopping rss_r service");
    execute_command(&session, "sudo systemctl stop rss_r")?;

    let remote_zip_path = upload_zip_to_tmp_dir(config, &session)?;

    info!("Check if zip contains expected files");
    let rss_r_exec_in_zip = Utf8PathBuf::from("rss_r/rss_r");
    let static_dir_in_zip = Utf8PathBuf::from("rss_r/static/");

    execute_command(
        &session,
        &format!(
            "unzip -l '{}' | grep -q '{}'",
            remote_zip_path, rss_r_exec_in_zip
        ),
    )
    .with_context(|| format!("Zip does not contain `{}`", rss_r_exec_in_zip))?;
    execute_command(
        &session,
        &format!(
            "unzip -l '{}' | grep -q '{}'",
            remote_zip_path, static_dir_in_zip
        ),
    )
    .with_context(|| format!("Zip does not contain `{}`", static_dir_in_zip))?;
    info!("Expected files found");

    // The old static directory needs removing to make sure there are no old files
    // left behind. Because the `unzip` command will only add or overwrite files.
    info!("Removing old static directory");
    let mut target_static_dir = config.rss_r_production_directory.clone();
    target_static_dir.push("static");
    // TODO (2024-09-08): Make this command not fail if the static dir is not there.
    execute_command(&session, &format!("sudo rm -r '{target_static_dir}'"))?;

    info!("Extracting rss_r exe and static directory");
    // `-j`: unzip only the files specified, do not create their parent directories.
    // `-o`: Overwrite files without prompting.
    execute_command(
        &session,
        &format!(
            "sudo unzip -j -o '{remote_zip_path}' '{rss_r_exec_in_zip}' -d {}",
            config.rss_r_production_directory
        ),
    )?;
    execute_command(
        &session,
        &format!(
            "sudo unzip -j -o '{remote_zip_path}' '{static_dir_in_zip}*' -d {target_static_dir}",
        ),
    )?;

    info!("Setting ownership to {}", config.rss_r_production_user);
    let mut target_rss_exe = config.rss_r_production_directory.clone();
    target_rss_exe.push("rss_r");
    execute_command(
        &session,
        &format!(
            "sudo chown '{}':'{}' '{}'",
            config.rss_r_production_user, config.rss_r_production_user, target_rss_exe
        ),
    )?;
    execute_command(
        &session,
        &format!(
            "sudo chown -R '{}':'{}' '{}'",
            config.rss_r_production_user, config.rss_r_production_user, target_static_dir
        ),
    )?;

    info!("Starting rss_r service");
    execute_command(&session, "sudo systemctl start rss_r")?;

    info!("Getting status of service");
    execute_command(&session, "systemctl status rss_r")?;

    Ok(())
}

fn deploy_to_test_dir(config: &Config) -> Result<()> {
    let session = connect_and_login(config)?;

    let remote_zip_path = upload_zip_to_tmp_dir(config, &session)?;

    info!("Unpacking package to `{}`", config.rss_r_target_test_dir);
    execute_command(
        &session,
        &format!("rm -rf '{}'", config.rss_r_target_test_dir),
    )?;
    execute_command(
        &session,
        &format!(
            "unzip '{}' -d '{}'",
            remote_zip_path, config.rss_r_target_test_dir
        ),
    )?;

    info!("Transferring app config file.");
    let mut config_file_target = config.rss_r_target_test_dir.clone();
    config_file_target.push("rss_r");
    config_file_target.push("persistence");

    execute_command(&session, &format!("mkdir -p '{}'", config_file_target))?;

    config_file_target.push("app_config.ron");

    upload_file(
        &session,
        &config.rss_r_test_config_file,
        &config_file_target,
    )?;

    info!("Upload complete.");

    Ok(())
}

/// Returns the path to the uploaded zip.
fn upload_zip_to_tmp_dir(config: &Config, session: &Session) -> Result<Utf8PathBuf> {
    info!("Uploading zip to temp directory");
    let package_name = config
        .rss_r_zip
        .file_name()
        .ok_or_eyre("Cannot upload file, path does not have file name.")?;
    let mut remote_temp_path = Utf8PathBuf::from(REMOTE_TEMP_DIR);
    remote_temp_path.push(package_name);

    upload_file(session, &config.rss_r_zip, &remote_temp_path)?;

    Ok(remote_temp_path)
}

fn run_test_rss_r(config: &Config, session: &Session) -> Result<()> {
    let mut exec_path = config.rss_r_target_test_dir.clone();
    // Top directory in the .zip should be rss_r.
    exec_path.push("rss_r");
    // Executable is also called rss_r.
    exec_path.push("rss_r");

    let mut working_dir = config.rss_r_target_test_dir.clone();
    working_dir.push("rss_r");

    info!("Running `{}`", exec_path);
    println!("----------");

    // Make sure to have the working directory be the same as the rss_r directory,
    // so that the program can locate the persistence and config files properly.
    execute_command(session, &format!("cd '{}'; '{}'", working_dir, exec_path))
}

fn connect_and_login(config: &Config) -> Result<Session> {
    let target = config.host_and_port();
    info!("Connecting to `{}`", target);

    let tcp = TcpStream::connect(&target)
        .with_context(|| format!("Could not connect to `{}`", target))?;
    let mut session = Session::new()?;

    session.set_tcp_stream(tcp);
    session.handshake()?;

    session.userauth_agent(&config.username)?;

    info!("Logged in as `{}`", config.username);

    Ok(session)
}

/// Executes a given command.
/// Prints the stdout and stderr output as it arrives.
/// Returns an error if the command had a non-zero exit code.
fn execute_command(session: &Session, command: &str) -> Result<()> {
    // We'll listen to Ctrl+c (SIGINT) while running a command.
    // So that we can gracefully shut it down.
    let mut signals = Signals::new([SIGINT])?;

    let mut channel = session.channel_session()?;
    // Will merge stdout and stderr data into stdout.
    channel.handle_extended_data(ExtendedData::Merge)?;

    channel.exec(command)?;

    while !channel.eof() {
        let mut bytes = [0; 32];

        let amount = channel.read(&mut bytes)?;
        stdout().write_all(&bytes[0..amount])?;

        stdout().flush()?;

        if signals.pending().next().is_some() {
            // Received interrupt signal.
            info!("Stopping remote command...");

            // Ask the remote to stop the command.
            // TODO (Wybe 2022-10-17): This does not work yet. How do we stop an ongoing command in this case?
            channel.send_eof()?;
            channel.close()?;
            break;
        }
    }

    channel.wait_close()?;
    let exit_code = channel.exit_status()?;

    if exit_code == 0 {
        Ok(())
    } else {
        Err(eyre!(
            "command `{}` failed with exit code `{}`",
            command,
            exit_code
        ))
    }
}

fn upload_file(session: &Session, file: &Utf8Path, remote_path: &Utf8Path) -> Result<()> {
    let mut local_file = File::open(file)?;
    let mut bytes = Vec::new();
    local_file.read_to_end(&mut bytes)?;

    info!("Uploading `{}` to `{}`", file, remote_path);

    let mut remote_file =
        session.scp_send(remote_path.as_std_path(), 0o644, bytes.len() as u64, None)?;

    remote_file.write_all(&bytes)?;
    remote_file.send_eof()?;
    remote_file.wait_eof()?;
    remote_file.close()?;
    remote_file.wait_close()?;

    Ok(())
}

fn configure_logging() -> Result<()> {
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
    .context("Could not start logger")
}
