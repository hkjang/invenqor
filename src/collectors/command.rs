use anyhow::{Context, Result};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn run(program: &str, args: &[&str], timeout: Duration) -> Result<CommandOutput> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("start {program}"))?;
    let mut stdout = child.stdout.take().context("capture command stdout")?;
    let mut stderr = child.stderr.take().context("capture command stderr")?;
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("poll command")? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            anyhow::bail!(
                "{program} exceeded its {} second timeout",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("{program} stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("{program} stderr reader panicked"))??;
    Ok(CommandOutput {
        success: status.success(),
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_without_a_shell() {
        let output = run("printf", &["hello"], Duration::from_secs(1)).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, b"hello");
    }

    #[test]
    fn terminates_a_command_at_its_deadline() {
        let error = run("sleep", &["1"], Duration::from_millis(10)).unwrap_err();
        assert!(error.to_string().contains("timeout"));
    }
}
