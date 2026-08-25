use anyhow::{Context, Result};
use invenqor_agent::config::{Config, ConfigAvailability};
use invenqor_agent::diagnose;
use invenqor_agent::identity;
use invenqor_agent::platform;
use invenqor_agent::scheduler::Agent;
use invenqor_agent::storage::StateStore;
use invenqor_agent::updater;
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

/// The flags the help text documents. Kept beside the parser so the two cannot
/// drift: a documented flag that the parser rejects is a bug a user meets first.
const HELP_FLAGS: &[&str] = &[
    "--once",
    "--diagnose",
    "--status",
    "--json",
    "--validate-config",
    "--apply-pending-update",
    "--check-update",
    "--update-now",
    "--print-default-config",
    "--help",
    "--version",
    // Windows only: the Service Control Manager starts the installed service
    // with this flag. It is accepted everywhere so the parser and the help text
    // never disagree by platform.
    "--service",
    // New SCM registrations use the more explicit internal spelling. Keep the
    // old switch above forever so an existing service can upgrade in place.
    "--service-run",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let service_options = match service_launch_options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("invenqor-agent: {error:#}");
            std::process::exit(2);
        }
    };
    #[cfg(not(windows))]
    let _ = &service_options;

    // The Service Control Manager expects the process to hand itself to the
    // dispatcher before doing anything else, and it will kill a service that
    // does not report RUNNING within its timeout. Everything else - including a
    // service binary invoked from a console - runs the ordinary path.
    #[cfg(windows)]
    {
        // Help and version do not need to touch machine state. All operational
        // commands resolve the protected marker so diagnostics and a console
        // update target the same custom service the installer registered.
        let informational = args.iter().any(|value| {
            matches!(
                value.as_str(),
                "--help" | "-h" | "--version" | "-V" | "--print-default-config"
            )
        });
        if service_options.run_under_scm || !informational {
            let config_path = argument_value(&args, "--config")
                .map(PathBuf::from)
                .unwrap_or_else(platform::default_config_path);
            let service_name = match invenqor_agent::service_identity::resolve_service_name(
                service_options.name.as_deref(),
                &config_path,
            )
            .and_then(|name| {
                invenqor_agent::windows_service::configure_service_name(&name)?;
                Ok(name)
            }) {
                Ok(name) => name,
                Err(error) => {
                    eprintln!("invenqor-agent: {error:#}");
                    std::process::exit(2);
                }
            };
            if service_options.run_under_scm {
                if invenqor_agent::windows_service::dispatch(service_body) {
                    return;
                }
                // Not actually started by the SCM: fall through and run in the
                // foreground so an internal service command copied from the SCM
                // can be debugged rather than silently doing nothing.
                eprintln!(
                    "invenqor-agent: {service_name} was not started by the service \
                     control manager; running in the foreground"
                );
            }
        }
    }
    std::process::exit(blocking_run());
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ServiceLaunchOptions {
    run_under_scm: bool,
    name: Option<String>,
}

fn service_launch_options(args: &[String]) -> Result<ServiceLaunchOptions> {
    reject_unknown_arguments(args)?;
    let names = argument_values(args, "--service-name")?;
    anyhow::ensure!(
        names.len() <= 1,
        "--service-name may be specified only once"
    );
    let name = names.into_iter().next();
    if let Some(value) = name.as_deref() {
        invenqor_agent::service_identity::validate_service_name(value)?;
    }
    Ok(ServiceLaunchOptions {
        run_under_scm: args
            .iter()
            .any(|value| value == "--service" || value == "--service-run"),
        name,
    })
}

/// Runs the agent on its own runtime and returns a process exit code.
fn blocking_run() -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("invenqor-agent: start runtime: {error}");
            return 1;
        }
    };
    match runtime.block_on(run()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("invenqor-agent: {error:#}");
            1
        }
    }
}

#[cfg(windows)]
fn service_body() -> i32 {
    blocking_run()
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
    let update_now = args.iter().any(|v| v == "--update-now");
    let diagnose_flag = args.iter().any(|v| v == "--diagnose");
    let status_flag = args.iter().any(|v| v == "--status");
    let json = args.iter().any(|v| v == "--json");
    let default_config = platform::default_config_path();
    let config_path = argument_value(&args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_config.clone());
    reject_unknown_arguments(&args)?;

    let availability = ConfigAvailability::inspect(&config_path);
    let config_present = availability.is_readable();
    // Only set for --diagnose and --status: what stopped the file from being
    // used, so the report can name it instead of the process exiting on the very
    // fault it was run to explain.
    let mut config_fault: Option<String> = None;
    let config = match availability {
        ConfigAvailability::Readable if diagnose_flag || status_flag => {
            match Config::load(&config_path) {
                Ok(config) => config,
                Err(error) => {
                    config_fault = Some(format!("{error:#}"));
                    Config::default()
                }
            }
        }
        ConfigAvailability::Readable => Config::load(&config_path)?,
        // --diagnose and --status exist to explain a broken installation, so they
        // must produce their report rather than exit on the very fault they are
        // there to name. They run on defaults and the report says why.
        ConfigAvailability::Unreadable if diagnose_flag || status_flag => {
            config_fault = Some(format!(
                "{} exists but this account cannot read it",
                config_path.display()
            ));
            Config::default()
        }
        // Running on built-in defaults because the file could not be *read* is
        // the failure that hides itself: the Agent collects into its queue,
        // never registers, and the only clue is a warning that says the file
        // was not found. Refuse to start instead, and name the fix.
        ConfigAvailability::Unreadable => anyhow::bail!(
            "cannot read config {path}: the file or a directory on the way to it \
             denies access to this account ({account}). The service account needs \
             read access:\n  \
             sudo chown root:invenqor-agent {parent} {path}\n  \
             sudo chmod 0750 {parent}\n  \
             sudo chmod 0640 {path}",
            path = config_path.display(),
            parent = config_path
                .parent()
                .map(|value| value.display().to_string())
                .unwrap_or_else(|| default_config.display().to_string()),
            account = current_account(),
        ),
        ConfigAvailability::Missing if config_path == default_config => Config::default(),
        ConfigAvailability::Missing => {
            anyhow::bail!("config file does not exist: {}", config_path.display())
        }
    };
    config.validate()?;
    if validate {
        println!("configuration is valid");
        return Ok(0);
    }
    if apply_update {
        match updater::apply_pending(&config)? {
            Some(version) => {
                record_applied_update(&config, &version);
                println!("applied invenqor-agent update {version}");
            }
            None => println!("no pending update"),
        }
        return Ok(0);
    }
    if status_flag {
        return print_status(&config, json);
    }
    if diagnose_flag {
        let report = diagnose::run(
            &config,
            &config_path,
            config_present,
            config_fault.as_deref(),
        )
        .await;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{}", report.render());
        }
        return Ok(if report.failed() { 1 } else { 0 });
    }

    init_logging(&config.agent.state_dir);
    log_panics();
    let run_marker = note_run_boundary(&config.agent.state_dir);
    if !config_present && config_path == default_config {
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
    if update_now {
        return update_in_one_step(&config, &identity.agent_id).await;
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
        let outcome = agent.run().await.map(|()| 0);
        // Only a run that returned - a stop the service manager asked for, or a
        // requested restart - clears the marker. A crash leaves it, and the next
        // start says so.
        if outcome.is_ok() {
            let _ = std::fs::remove_file(&run_marker);
        }
        outcome
    }
}

/// Checks, verifies, stages and installs in one command. The staged-then-applied
/// split exists because the collector runs unprivileged and only a root helper may
/// replace the binary; when an operator is already root, making them run two
/// commands in the right order is needless ceremony.
async fn update_in_one_step(config: &Config, agent_id: &str) -> Result<i32> {
    let staged = updater::check_and_stage(config, agent_id).await?;
    match &staged {
        Some(version) => println!("verified and staged invenqor-agent {version}"),
        None => {
            println!("already up to date on channel {}", config.updates.channel);
            return Ok(0);
        }
    }
    match updater::apply_pending(config) {
        Ok(Some(version)) => {
            record_applied_update(config, &version);
            println!(
                "installed invenqor-agent {version} at {}",
                config.updates.install_path.display()
            );
            println!(
                "service restart command: {}",
                invenqor_agent::platform::restart_command()
            );
            Ok(0)
        }
        Ok(None) => {
            println!("nothing to install");
            Ok(0)
        }
        Err(error) => {
            // Staging succeeded, so the download and signature were fine; this is
            // a permission or self-test failure and the running agent is untouched.
            eprintln!("could not install the staged update: {error:#}");
            eprintln!(
                "the running agent is unchanged. Install it as root with: \
                 invenqor-agent --config {} --apply-pending-update",
                config_path_hint(config)
            );
            Ok(3)
        }
    }
}

fn config_path_hint(config: &Config) -> String {
    let _ = config;
    platform::default_config_path().display().to_string()
}

/// Records the applied version in the status report so an operator can confirm
/// the update without reading the journal.
fn record_applied_update(config: &Config, version: &str) {
    let Ok(store) = StateStore::open(&config.agent.state_dir, config.agent.max_queue_bytes) else {
        return;
    };
    let Some(mut status) = store.read_status() else {
        return;
    };
    status.record_update_applied(version, invenqor_agent::model::unix_time());
    let _ = store.write_status(&status);
}

/// The account the process is running as, so a permission message names who was
/// denied rather than leaving the reader to guess.
fn current_account() -> String {
    // No libc dependency: the effective user is what the kernel reports here.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("Uid:").map(|rest| {
                    let effective = rest.split_whitespace().nth(1).unwrap_or("?");
                    format!("uid {effective}")
                })
            })
        })
        .unwrap_or_else(|| "the current account".to_string())
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
        println!(
            "  updates       {} · 실행 {}{}",
            if status.updates.enabled {
                "자동"
            } else {
                "비활성"
            },
            status.updates.running_version,
            match (
                &status.updates.staged_version,
                &status.updates.applied_version
            ) {
                (Some(staged), _) => format!(" · 대기 {staged}"),
                (None, Some(applied)) => format!(" · 최근 적용 {applied}"),
                _ => String::new(),
            }
        );
        if let Some(error) = &status.updates.last_error {
            println!(
                "                update error {}: {}",
                error.code, error.detail
            );
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

fn argument_values(args: &[String], name: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let joined_prefix = format!("{name}=");
    let mut index = 0;
    while index < args.len() {
        if args[index] == name {
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .with_context(|| format!("{name} requires a value"))?;
            values.push(value.clone());
            index += 2;
            continue;
        }
        if let Some(value) = args[index].strip_prefix(&joined_prefix) {
            anyhow::ensure!(!value.is_empty(), "{name} requires a value");
            values.push(value.to_string());
        }
        index += 1;
    }
    Ok(values)
}

fn reject_unknown_arguments(args: &[String]) -> Result<()> {
    let mut skip = false;
    for arg in args {
        if skip {
            anyhow::ensure!(
                !arg.starts_with("--"),
                "option requires a value before {arg}"
            );
            skip = false;
            continue;
        }
        match arg.as_str() {
            // The short forms of the two flags that have one. Everything else is
            // accepted straight from the documented list below, so a flag cannot
            // be offered in --help and refused here.
            "-h" | "-V" => {}
            "--config" | "--service-name" => skip = true,
            value if value.starts_with("--config=") => {}
            value if value.starts_with("--service-name=") => {}
            value if HELP_FLAGS.contains(&value) => {}
            value => anyhow::bail!("unknown argument: {value}"),
        }
    }
    anyhow::ensure!(!skip, "argument requires a value");
    Ok(())
}

/// Starts logging, and on Windows also writes to a file beside the state.
///
/// A Windows service's standard error leads nowhere, so everything logged while
/// running as a service used to be discarded - which is how a service that never
/// completed a collection could look healthy from the outside. The file is the
/// only place that failure is visible.
fn init_logging(state_dir: &Path) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("invenqor_agent=info"));
    #[cfg(windows)]
    {
        if let Some(file) = invenqor_agent::logfile::LogFile::open(state_dir) {
            let path = file.path().to_path_buf();
            // Leaked deliberately: the writer must outlive every span for the
            // lifetime of the process, and the process owns exactly one.
            let writer: &'static invenqor_agent::logfile::LogFile = Box::leak(Box::new(file));
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(move || writer)
                .init();
            tracing::info!(log = %path.display(), "agent log file opened");
            return;
        }
        eprintln!(
            "invenqor-agent: could not open a log file under {}; logging to stderr only, \
             which a Windows service discards",
            state_dir.display()
        );
    }
    let _ = state_dir;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Routes panics into the log.
///
/// The default hook writes its message to standard error, which a Windows service
/// does not have. Back when the release profile also aborted on panic, that
/// combination killed the process leaving nothing behind at all: the log showed
/// the four start-up lines, then silence, then the service manager starting it
/// again. The profile now unwinds, so a panicking collector is caught and
/// reported as that collector's error - but the panic message itself still only
/// reaches the log through this hook.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|value| format!("{}:{}", value.file(), value.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|value| (*value).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_string());
        tracing::error!(
            location = %location,
            panic = %message,
            "the agent panicked and is aborting; this is a bug"
        );
        default(info);
    }));
}

/// Records that this run started, and reports whether the previous one ended.
///
/// A process that dies without unwinding leaves the marker behind. Finding it at
/// start-up is what turns "the log begins again every twenty-eight seconds" -
/// which an operator has to notice and interpret - into a statement.
fn note_run_boundary(state_dir: &Path) -> std::path::PathBuf {
    let marker = state_dir.join("running.marker");
    if marker.exists() {
        tracing::warn!(
            marker = %marker.display(),
            "the previous run did not shut down cleanly; if this repeats every few \
             seconds the agent is crashing and being restarted by the service manager"
        );
    }
    if let Err(error) = std::fs::write(&marker, std::process::id().to_string()) {
        tracing::debug!(error = %error, "could not write the run marker");
    }
    marker
}

fn print_help() {
    let default_config = platform::default_config_path().display().to_string();
    // The flag exists on every build so the parser never disagrees with the help
    // text, but it only means anything where there is a service manager to be
    // dispatched to.
    let service_help = if cfg!(windows) {
        "\n  --service-name NAME     Windows service identity (normally auto-discovered)"
    } else {
        ""
    };
    println!(
        "Portable Linux and Windows asset inventory agent

Usage: invenqor-agent [OPTIONS]

Options:
  --config PATH           Configuration file (default: {default_config})
  --once                  Collect once, attempt delivery, and print JSON
                          (exit 2 when delivery to a configured Server fails)
  --diagnose              Check registration and connectivity without changing
                          state, and exit non-zero on the first blocking fault
  --status                Print the persisted registration and delivery status
  --json                  Emit --diagnose or --status output as JSON
  --validate-config       Validate configuration and exit
  --apply-pending-update  Root helper: verify and atomically apply staged update
  --check-update          Check, verify, and stage an available signed update
  --update-now            Check, stage and install in one step (needs write
                          access to the install path; the running agent is left
                          untouched if the new binary fails its self-test)
  --print-default-config  Print a complete default configuration
  -V, --version           Print version
  -h, --help              Print help{service_help}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every flag the help text offers must also be accepted. These two lists sat
    /// apart, and `--update-now` shipped documented but rejected.
    #[test]
    fn every_documented_flag_is_accepted() {
        let mut help = Vec::new();
        for line in HELP_FLAGS {
            help.push((*line).to_string());
        }
        for flag in &help {
            reject_unknown_arguments(&[flag.clone()]).unwrap_or_else(|error| {
                panic!("the help text documents {flag} but it is rejected: {error}")
            });
        }
        reject_unknown_arguments(&[
            "--service-name".to_string(),
            "Invenqor Agent West-1".to_string(),
        ])
        .unwrap();
    }

    #[test]
    fn unknown_flags_are_still_refused() {
        assert!(reject_unknown_arguments(&["--wat".to_string()]).is_err());
        // A path-taking flag must not swallow the end of the arguments.
        assert!(reject_unknown_arguments(&["--config".to_string()]).is_err());
        reject_unknown_arguments(&["--config".to_string(), "/tmp/x.toml".to_string()]).unwrap();
        reject_unknown_arguments(&["--config=/tmp/x.toml".to_string()]).unwrap();
        assert!(reject_unknown_arguments(&["--service-name".to_string()]).is_err());
    }

    #[test]
    fn argument_value_reads_both_forms() {
        let separated = vec!["--config".to_string(), "/etc/a.toml".to_string()];
        assert_eq!(
            argument_value(&separated, "--config").as_deref(),
            Some("/etc/a.toml")
        );
        let joined = vec!["--config=/etc/b.toml".to_string()];
        assert_eq!(
            argument_value(&joined, "--config").as_deref(),
            Some("/etc/b.toml")
        );
        assert!(argument_value(&[], "--config").is_none());
    }

    #[test]
    fn service_launch_accepts_legacy_and_new_scm_switches() {
        let legacy = service_launch_options(&["--service".to_string()]).unwrap();
        assert!(legacy.run_under_scm);
        assert_eq!(legacy.name, None);

        let current = service_launch_options(&[
            "--service-run".to_string(),
            "--service-name".to_string(),
            "Invenqor Agent West-1".to_string(),
        ])
        .unwrap();
        assert!(current.run_under_scm);
        assert_eq!(current.name.as_deref(), Some("Invenqor Agent West-1"));
    }

    #[test]
    fn service_name_is_single_valued_and_cannot_consume_another_flag() {
        assert!(service_launch_options(&[
            "--service-name".to_string(),
            "one".to_string(),
            "--service-name=two".to_string(),
        ])
        .is_err());
        assert!(
            service_launch_options(&["--service-name".to_string(), "--diagnose".to_string(),])
                .is_err()
        );
        assert!(service_launch_options(&[
            "--service-name=invenqor-agent\" --config C:\\attacker.toml".to_string(),
        ])
        .is_err());
    }
}
