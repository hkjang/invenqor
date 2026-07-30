//! Service Control Manager integration.
//!
//! Windows will not simply run a process in the background: a service has to
//! register a control handler and report its state transitions, or the SCM marks
//! it as failed to start and kills it. This module is that contract, and the
//! restart path the updater needs.
//!
//! Restarting after an update is the interesting part. A service cannot stop and
//! start itself - the SCM will not process a start for a service that is still
//! stopping - so the accepted pattern is to stop with a failure exit code and let
//! the SCM's recovery action bring it back. The installer configures that
//! recovery action; `request_restart` triggers it.

use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const SERVICE_NAME: &str = "invenqor-agent";

/// The exit code the service reports when it wants the SCM to restart it. Any
/// non-zero code triggers the configured recovery action; a distinct value makes
/// an intentional restart-for-update tellable from a crash in the event log.
pub const EXIT_CODE_RESTART_FOR_UPDATE: u32 = 0x2A;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static EXIT_CODE: AtomicU32 = AtomicU32::new(0);
static SERVICE_HANDLE: AtomicU32 = AtomicU32::new(0);

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
    ) -> u32;
    fn SetServiceStatus(handle: u32, status: *mut ServiceStatus) -> i32;
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

/// True once the SCM has asked the service to stop, or the updater has asked for
/// a restart. The collection loop checks it so a stop is honoured promptly rather
/// than at the end of a fifteen-minute sleep.
pub fn stop_requested() -> bool {
    STOP_REQUESTED.load(Ordering::SeqCst)
}

/// Asks the SCM to restart this service, by stopping with the recovery-triggering
/// exit code. Used after an update has been swapped into place: the new binary is
/// on disk but this process is still the old one.
pub fn request_restart() {
    EXIT_CODE.store(EXIT_CODE_RESTART_FOR_UPDATE, Ordering::SeqCst);
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
    unsafe {
        SetServiceStatus(handle, &mut status);
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
    let handle = RegisterServiceCtrlHandlerW(wide(SERVICE_NAME).as_ptr(), control_handler);
    if handle == 0 {
        return;
    }
    SERVICE_HANDLE.store(handle, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0, 15_000);
    report(SERVICE_RUNNING, 0, 0);
    let code = match BODY.get() {
        Some(body) => body(),
        None => 1,
    };
    let exit_code = if code == 0 {
        EXIT_CODE.load(Ordering::SeqCst)
    } else {
        code as u32
    };
    report(SERVICE_STOPPED, exit_code, 0);
}

/// Hands the process to the SCM. Returns false when this process was not started
/// as a service, so the caller can fall back to running in the foreground.
pub fn dispatch(body: ServiceBody) -> bool {
    let _ = BODY.set(body);
    let name = wide(SERVICE_NAME);
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
    let service = unsafe {
        OpenServiceW(
            manager,
            wide(SERVICE_NAME).as_ptr(),
            SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS,
        )
    };
    if service == 0 {
        unsafe { CloseServiceHandle(manager) };
        anyhow::bail!(
            "the {SERVICE_NAME} service is not installed, so there is nothing to restart"
        );
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
        anyhow::ensure!(
            unsafe { StartServiceW(service, 0, std::ptr::null()) } != 0,
            "the service stopped but did not start again; start it with \
             `Start-Service {SERVICE_NAME}` and check the event log"
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
