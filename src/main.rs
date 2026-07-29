use anyhow::Result;
use invenqor_agent::config::Config;
use invenqor_agent::diagnose;
use invenqor_agent::identity;
use invenqor_agent::scheduler::Agent;
use invenqor_agent::storage::StateStore;
use invenqor_agent::updater;
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

const DEFAULT_CONFIG: &str = "/etc/invenqor-agent/config.toml";

#[tokio::main]
async fn main() {
    match run().await {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("invenqor-agent: {error:#}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|v| v == "--help" || v == "-h") {
        print_help();
        return Ok(0);
    }
    if args.iter().any(|v| v == "--version" || v == "-V") {
        println!("invenqor-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if args.iter().any(|v| v == "--print-default-config") {
        println!("{}", toml::to_string_pretty(&Config::default())?);
        return Ok(0);
    }

    let once = args.iter().any(|v| v == "--once");
    let validate = args.iter().any(|v| v == "--validate-config");
    let apply_update = args.iter().any(|v| v == "--apply-pending-update");
    let check_update = args.iter().any(|v| v == "--check-update");
    let diagnose_flag = args.iter().any(|v| v == "--diagnose");
    let status_flag = args.iter().any(|v| v == "--status");
    let json = args.iter().any(|v| v == "--json");
    let config_path = argument_value(&args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG));
    reject_unknown_arguments(&args)?;

    let config_present = config_path.exists();
    let config = if config_present {
        Config::load(&config_path)?
    } else if config_path == Path::new(DEFAULT_CONFIG) {
        Config::default()
    } else {
        anyhow::bail!("config file does not exist: {}", config_path.display());
    };
    config.validate()?;
    if validate {
        println!("configuration is valid");
        return Ok(0);
    }
    if apply_update {
        match updater::apply_pending(&config)? {
            Some(version) => println!("applied invenqor-agent update {version}"),
            None => println!("no pending update"),
        }
        return Ok(0);
    }
    if status_flag {
        return print_status(&config, json);
    }
    if diagnose_flag {
        let report = diagnose::run(&config, &config_path, config_present).await;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{}", report.render());
        }
        return Ok(if report.failed() { 1 } else { 0 });
    }

    init_logging();
    if !config_present && config_path == Path::new(DEFAULT_CONFIG) {
        tracing::warn!(
            config = %config_path.display(),
            "no configuration file was found; built-in defaults are in use and no Server is configured"
        );
    }
    if config
        .server
        .url
        .as_deref()
        .is_some_and(|value| value.starts_with("http://"))
    {
        tracing::warn!(
            "Agent transport is using unencrypted HTTP; use HTTPS when traffic crosses a trusted network boundary"
        );
    }
    let identity = identity::load_or_create(&config.agent.state_dir)?;
    tracing::info!(
        agent_id = %identity.agent_id,
        machine_id = ?identity.machine_id,
        dmi_product_uuid = ?identity.dmi_product_uuid,
        "agent identity loaded"
    );
    if check_update {
        match updater::check_and_stage(&config, &identity.agent_id).await? {
            Some(version) => println!("staged invenqor-agent update {version}"),
            None => println!("no update available"),
        }
        return Ok(0);
    }
    let mut agent = Agent::new(config, identity, &config_path)?;
    if once {
        let snapshot = agent.collect_once().await?;
        // The snapshot always reaches stdout so an offline collection stays
        // useful, but the exit code has to reveal a failed handover; a silent
        // success here is what hid broken registrations from installers.
        let delivery = agent.drain_queue().await;
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        match delivery {
            Ok(_) => Ok(0),
            Err(error) => {
                tracing::warn!(
                    error = %format!("{error:#}"),
                    summary = %agent.status().headline(),
                    "the collected snapshot stays queued because it could not be delivered"
                );
                Ok(2)
            }
        }
    } else {
        agent.run().await.map(|()| 0)
    }
}

fn print_status(config: &Config, json: bool) -> Result<i32> {
    let store = StateStore::open(&config.agent.state_dir, config.agent.max_queue_bytes)?;
    let Some(status) = store.read_status() else {
        println!(
            "no status report exists yet at {}; start the Agent or run --diagnose",
            store.status_path().display()
        );
        return Ok(1);
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!(
            "invenqor-agent {} on {}",
            status.agent_version, status.hostname
        );
        println!("  updated       {}", status.updated_at_utc);
        println!(
            "  server.url    {}",
            status.server_url.as_deref().unwrap_or("(not configured)")
        );
        println!(
            "  registration  {} ({})",
            status.enrollment.state, status.enrollment.summary
        );
        println!(
            "  queue         {} event(s), {} of {} bytes",
            status.queue.pending_events, status.queue.bytes, status.queue.limit_bytes
        );
        println!(
            "  delivered     {} event(s), last success {}",
            status.delivery.delivered_events,
            status
                .delivery
                .last_success_at_utc
                .as_deref()
                .unwrap_or("never")
        );
        for error in [
            status.enrollment.last_error.as_ref(),
            status.delivery.last_error.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            println!(
                "  last error    {} during {} at {}",
                error.code, error.operation, error.occurred_at_utc
            );
            println!("                {}", error.detail);
            if let Some(request_id) = &error.request_id {
                println!("                server request_id {request_id}");
            }
            println!("                fix: {}", error.remediation);
        }
        println!("  summary       {}", status.headline());
    }
    Ok(if status.degraded() { 1 } else { 0 })
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
            "--once"
            | "--validate-config"
            | "--apply-pending-update"
            | "--check-update"
            | "--diagnose"
            | "--status"
            | "--json" => {}
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
                          (exit 2 when delivery to a configured Server fails)
  --diagnose              Check registration and connectivity without changing
                          state, and exit non-zero on the first blocking fault
  --status                Print the persisted registration and delivery status
  --json                  Emit --diagnose or --status output as JSON
  --validate-config       Validate configuration and exit
  --apply-pending-update  Root helper: verify and atomically apply staged update
  --check-update          Check, verify, and stage an available signed update
  --print-default-config  Print a complete default configuration
  -V, --version           Print version
  -h, --help              Print help"
    );
}
