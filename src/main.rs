use anyhow::Result;
use invenqor_agent::config::Config;
use invenqor_agent::identity;
use invenqor_agent::scheduler::Agent;
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

const DEFAULT_CONFIG: &str = "/etc/invenqor-agent/config.toml";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("invenqor-agent: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|v| v == "--help" || v == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|v| v == "--version" || v == "-V") {
        println!("invenqor-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|v| v == "--print-default-config") {
        println!("{}", toml::to_string_pretty(&Config::default())?);
        return Ok(());
    }

    let once = args.iter().any(|v| v == "--once");
    let validate = args.iter().any(|v| v == "--validate-config");
    let config_path = argument_value(&args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG));
    reject_unknown_arguments(&args)?;

    let config = if config_path.exists() {
        Config::load(&config_path)?
    } else if config_path == Path::new(DEFAULT_CONFIG) {
        Config::default()
    } else {
        anyhow::bail!("config file does not exist: {}", config_path.display());
    };
    config.validate()?;
    if validate {
        println!("configuration is valid");
        return Ok(());
    }

    init_logging();
    let identity = identity::load_or_create(&config.agent.state_dir)?;
    tracing::info!(
        agent_id = %identity.agent_id,
        machine_id = ?identity.machine_id,
        dmi_product_uuid = ?identity.dmi_product_uuid,
        "agent identity loaded"
    );
    let agent = Agent::new(config, identity)?;
    if once {
        let snapshot = agent.collect_once().await?;
        if let Err(error) = agent.drain_queue().await {
            tracing::warn!(error = %error, "queued snapshot for a later retry");
        }
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        Ok(())
    } else {
        agent.run().await
    }
}

fn argument_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .or_else(|| {
            args.iter()
                .find_map(|arg| arg.strip_prefix(&format!("{name}=")).map(str::to_string))
        })
}

fn reject_unknown_arguments(args: &[String]) -> Result<()> {
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        match arg.as_str() {
            "--once" | "--validate-config" => {}
            "--config" => skip = true,
            value if value.starts_with("--config=") => {}
            value => anyhow::bail!("unknown argument: {value}"),
        }
    }
    anyhow::ensure!(!skip, "--config requires a path");
    Ok(())
}

fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("invenqor_agent=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn print_help() {
    println!(
        "Portable Linux asset inventory agent

Usage: invenqor-agent [OPTIONS]

Options:
  --config PATH           Configuration file (default: {DEFAULT_CONFIG})
  --once                  Collect once, attempt delivery, and print JSON
  --validate-config       Validate configuration and exit
  --print-default-config  Print a complete default configuration
  -V, --version           Print version
  -h, --help              Print help"
    );
}
