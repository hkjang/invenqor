//! Service Control Manager integration.
//!
//! Windows will not simply run a process in the background: a service has to
//! register a control handler and report its state transitions, or the SCM marks
//! it as failed to start and kills it. This module is that contract, and the
//! restart path the updater needs.
//!
//! Restarting after an update is the interesting part. A service cannot stop and
//! start itself - the SCM will not process a start for a service that is still
//! stopping - so the process terminates without reporting SERVICE_STOPPED and
//! lets the SCM's recovery action bring it back. This works immediately after a
//! fresh install: SERVICE_FAILURE_ACTIONS_FLAG is documented to take effect only
//! after the next system start, while unexpected process termination triggers
//! recovery independently of that flag.

use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::OnceLock;

pub const SERVICE_NAME: &str = crate::service_identity::DEFAULT_SERVICE_NAME;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);
static ACTIVE_SERVICE_NAME: OnceLock<String> = OnceLock::new();
/// SERVICE_STATUS_HANDLE is a handle - pointer-sized. Declaring it as a 32-bit
/// value truncated it on x64, so SetServiceStatus was called with a corrupted
/// handle, every status report failed, the Service Control Manager never saw
/// RUNNING, and it terminated the service as having failed to start. The agent
/// never reached its first collection, which is why nothing was queued and
/// nothing registered while --diagnose reported a clear path to the Server.
static SERVICE_HANDLE: AtomicIsize = AtomicIsize::new(0);

const SERVICE_WIN32_OWN_PROCESS: u32 = 0x0000_0010;
const SERVICE_ACCEPT_STOP: u32 = 0x0000_0001;
const SERVICE_ACCEPT_SHUTDOWN: u32 = 0x0000_0004;
const SERVICE_START_PENDING: u32 = 0x0000_0002;
const SERVICE_RUNNING: u32 = 0x0000_0004;
const SERVICE_STOP_PENDING: u32 = 0x0000_0003;
const SERVICE_STOPPED: u32 = 0x0000_0001;
const SERVICE_CONTROL_STOP: u32 = 0x0000_0001;
const SERVICE_CONTROL_SHUTDOWN: u32 = 0x0000_0005;
const ERROR_SERVICE_SPECIFIC_ERROR: u32 = 1066;

#[repr(C)]
struct ServiceStatus {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    check_point: u32,
    wait_hint: u32,
}

#[repr(C)]
struct ServiceTableEntryW {
    name: *const u16,
    entry: Option<unsafe extern "system" fn(u32, *mut *mut u16)>,
}

#[link(name = "advapi32")]
extern "system" {
    fn StartServiceCtrlDispatcherW(table: *const ServiceTableEntryW) -> i32;
    fn RegisterServiceCtrlHandlerW(
        name: *const u16,
        handler: unsafe extern "system" fn(u32),
    ) -> isize;
    fn SetServiceStatus(handle: isize, status: *mut ServiceStatus) -> i32;
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> isize;
    fn OpenServiceW(manager: isize, name: *const u16, access: u32) -> isize;
    fn CloseServiceHandle(handle: isize) -> i32;
    fn ControlService(service: isize, control: u32, status: *mut ServiceStatus) -> i32;
    fn StartServiceW(service: isize, argc: u32, argv: *const *const u16) -> i32;
    fn QueryServiceStatus(service: isize, status: *mut ServiceStatus) -> i32;
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Sets the service identity before the SCM dispatcher or any diagnostic query
/// is opened. Main resolves this from the explicit service command line or the
/// protected marker next to config.toml. A second, different value indicates an
/// internal bootstrap error and is refused rather than querying the wrong SCM
/// object.
pub fn configure_service_name(value: &str) -> Result<()> {
    crate::service_identity::validate_service_name(value)?;
    if let Some(current) = ACTIVE_SERVICE_NAME.get() {
        anyhow::ensure!(
            current == value,
            "Windows service identity is already configured as {current}"
        );
        return Ok(());
    }
    ACTIVE_SERVICE_NAME
        .set(value.to_string())
        .map_err(|_| anyhow::anyhow!("configure Windows service identity"))
}

/// The service name used consistently by dispatch, status diagnostics and the
/// external restart path. The default preserves every pre-v0.2.15 install whose
/// SCM command line only contains the legacy `--service` switch.
pub fn service_name() -> &'static str {
    ACTIVE_SERVICE_NAME
        .get()
        .map(String::as_str)
        .unwrap_or(SERVICE_NAME)
}

/// True once the SCM has asked the service to stop, or the updater has asked for
/// a restart. The collection loop checks it so a stop is honoured promptly rather
/// than at the end of a fifteen-minute sleep.
pub fn stop_requested() -> bool {
    STOP_REQUESTED.load(Ordering::SeqCst)
}

/// Asks the SCM to restart this service. The service callback deliberately does
/// not report SERVICE_STOPPED for this path, so recovery is triggered even on a
/// fresh install before SERVICE_FAILURE_ACTIONS_FLAG becomes effective.
pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::SeqCst);
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

/// True when this process was started by the SCM rather than from a console.
pub fn started_by_service_manager() -> bool {
    SERVICE_HANDLE.load(Ordering::SeqCst) != 0
}

fn report(state: u32, exit_code: u32, wait_hint: u32) {
    let handle = SERVICE_HANDLE.load(Ordering::SeqCst);
    if handle == 0 {
        return;
    }
    let mut status = ServiceStatus {
        service_type: SERVICE_WIN32_OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        },
        // A service-specific code has to be flagged as such or the SCM reads the
        // number as a Win32 error and the event log says something untrue.
        win32_exit_code: if exit_code == 0 {
            0
        } else {
            ERROR_SERVICE_SPECIFIC_ERROR
        },
        service_specific_exit_code: exit_code,
        check_point: 0,
        wait_hint,
    };
    if unsafe { SetServiceStatus(handle, &mut status) } == 0 {
        // Nothing can be done about it here, but it must not pass unrecorded:
        // this is the failure that makes a service look like it never started.
        tracing::error!(
            state,
            handle,
            "SetServiceStatus was rejected by the service control manager"
        );
    }
}

unsafe extern "system" fn control_handler(control: u32) {
    if control == SERVICE_CONTROL_STOP || control == SERVICE_CONTROL_SHUTDOWN {
        // 30 seconds: enough for the current collection cycle to finish writing
        // its queue entry, which is what keeps at-least-once delivery honest.
        report(SERVICE_STOP_PENDING, 0, 30_000);
        STOP_REQUESTED.store(true, Ordering::SeqCst);
    }
}

/// The body a service runs. Returns the process exit code.
type ServiceBody = fn() -> i32;
static BODY: std::sync::OnceLock<ServiceBody> = std::sync::OnceLock::new();

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
    let name = service_name();
    let handle = RegisterServiceCtrlHandlerW(wide(name).as_ptr(), control_handler);
    if handle == 0 {
        tracing::error!(
            service_name = name,
            "RegisterServiceCtrlHandlerW failed; the service cannot report its state"
        );
        return;
    }
    SERVICE_HANDLE.store(handle, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0, 15_000);
    report(SERVICE_RUNNING, 0, 0);
    let code = match BODY.get() {
        Some(body) => body(),
        None => 1,
    };
    if should_report_stopped(RESTART_REQUESTED.load(Ordering::SeqCst)) {
        report(SERVICE_STOPPED, code.max(0) as u32, 0);
    } else {
        // Returning from the callback lets the service process exit. Reporting
        // even a non-zero SERVICE_STOPPED here would rely on failureflag=1,
        // which Windows does not honor until the next boot.
        tracing::info!("update restart requested; leaving service state running so SCM recovery restarts the new binary");
    }
}

fn should_report_stopped(restart_requested: bool) -> bool {
    !restart_requested
}

/// Hands the process to the SCM. Returns false when this process was not started
/// as a service, so the caller can fall back to running in the foreground.
pub fn dispatch(body: ServiceBody) -> bool {
    let _ = BODY.set(body);
    let name = wide(service_name());
    let table = [
        ServiceTableEntryW {
            name: name.as_ptr(),
            entry: Some(service_main),
        },
        ServiceTableEntryW {
            name: std::ptr::null(),
            entry: None,
        },
    ];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) != 0 }
}

/// Stops and starts the service through the SCM. Used by an operator running
/// `--update-now` from an elevated console: the swap is done but the service is
/// still executing the old binary.
pub fn restart_service_externally() -> Result<()> {
    const SC_MANAGER_CONNECT: u32 = 0x0001;
    const SERVICE_START: u32 = 0x0010;
    const SERVICE_STOP: u32 = 0x0020;
    const SERVICE_QUERY_STATUS: u32 = 0x0004;

    let manager = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    anyhow::ensure!(
        manager != 0,
        "could not open the service control manager; run this from an elevated console"
    );
    let name = service_name();
    let service = unsafe {
        OpenServiceW(
            manager,
            wide(name).as_ptr(),
            SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS,
        )
    };
    if service == 0 {
        unsafe { CloseServiceHandle(manager) };
        anyhow::bail!("the {name} service is not installed, so there is nothing to restart");
    }
    let result = (|| -> Result<()> {
        let mut status = blank_status();
        unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) };
        // Wait for the stop to complete; starting a stopping service fails.
        for _ in 0..60 {
            let mut current = blank_status();
            if unsafe { QueryServiceStatus(service, &mut current) } == 0 {
                break;
            }
            if current.current_state == SERVICE_STOPPED {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let restart_command = crate::platform::restart_command();
        anyhow::ensure!(
            unsafe { StartServiceW(service, 0, std::ptr::null()) } != 0,
            "the service stopped but did not start again; run `{restart_command}` \
             and check the event log"
        );
        Ok(())
    })();
    unsafe {
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
    }
    result.context("restart the Invenqor agent service")
}

fn blank_status() -> ServiceStatus {
    ServiceStatus {
        service_type: 0,
        current_state: 0,
        controls_accepted: 0,
        win32_exit_code: 0,
        service_specific_exit_code: 0,
        check_point: 0,
        wait_hint: 0,
    }
}

/// The state the Service Control Manager reports for the installed service.
///
/// `--diagnose` needs this because everything it checked before was about the
/// path from this host to the Server - and all of it can pass while the service
/// itself is stopped, which is exactly the case that looked healthy and was not.
pub fn installed_service_state() -> ServiceQuery {
    const SC_MANAGER_CONNECT: u32 = 0x0001;
    const SERVICE_QUERY_STATUS: u32 = 0x0004;
    const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;

    let manager = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if manager == 0 {
        return ServiceQuery::Unknown;
    }
    let service =
        unsafe { OpenServiceW(manager, wide(service_name()).as_ptr(), SERVICE_QUERY_STATUS) };
    if service == 0 {
        let missing = unsafe { GetLastError() } == ERROR_SERVICE_DOES_NOT_EXIST;
        unsafe { CloseServiceHandle(manager) };
        return if missing {
            ServiceQuery::NotInstalled
        } else {
            ServiceQuery::Unknown
        };
    }
    let mut status = blank_status();
    let queried = unsafe { QueryServiceStatus(service, &mut status) } != 0;
    unsafe {
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
    }
    if !queried {
        return ServiceQuery::Unknown;
    }
    ServiceQuery::Known {
        state: match status.current_state {
            1 => "stopped",
            2 => "start_pending",
            3 => "stop_pending",
            4 => "running",
            5 => "continue_pending",
            6 => "pause_pending",
            7 => "paused",
            _ => "unknown",
        },
        // A service that failed to start leaves its reason here, and it is the
        // first thing worth reporting when nothing is being collected.
        exit_code: if status.win32_exit_code == ERROR_SERVICE_SPECIFIC_ERROR {
            status.service_specific_exit_code
        } else {
            status.win32_exit_code
        },
    }
}

pub enum ServiceQuery {
    Known {
        state: &'static str,
        exit_code: u32,
    },
    NotInstalled,
    /// The SCM could not be asked - not running as a privileged account, most
    /// likely. Reporting that as "stopped" would be worse than saying nothing.
    Unknown,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
}

#[cfg(test)]
mod tests {
    use super::should_report_stopped;

    #[test]
    fn intentional_update_restart_never_reports_a_clean_stopped_state() {
        assert!(should_report_stopped(false));
        assert!(!should_report_stopped(true));
    }
}
