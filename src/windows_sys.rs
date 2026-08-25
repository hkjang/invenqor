//! The Windows system interfaces the Agent reads, and nothing else.
//!
//! Inventory on Windows comes from three places: the registry, a handful of
//! Win32 calls, and the Service Control Manager. This module wraps exactly what
//! the collectors need, so the collectors stay readable and every unsafe block
//! is in one file where it can be reviewed.
//!
//! WMI is deliberately not used. It is the usual way to read this data, but it
//! costs a COM apartment, a WQL parser and a service dependency (Winmgmt) that
//! is itself a common source of "the agent hangs on some hosts" - and every
//! value below is available without it. The firmware UUID, normally taken from
//! Win32_ComputerSystemProduct, is parsed out of the raw SMBIOS table instead.
#![allow(clippy::missing_safety_doc)]

use std::collections::BTreeMap;
use std::ffi::{c_void, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};

pub const HKEY_LOCAL_MACHINE: isize = -2147483646; // 0x8000_0002
const IF_OPER_STATUS_UP: u32 = 1;
pub const HKEY_USERS: isize = -2147483645; // 0x8000_0003

const ERROR_SUCCESS: i32 = 0;
const ERROR_MORE_DATA: i32 = 234;
const KEY_READ: u32 = 0x2_0019;
const KEY_WOW64_64KEY: u32 = 0x0100;
const KEY_WOW64_32KEY: u32 = 0x0200;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_DWORD: u32 = 4;
const REG_QWORD: u32 = 11;

type Hkey = isize;

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        key: Hkey,
        subkey: *const u16,
        options: u32,
        desired: u32,
        result: *mut Hkey,
    ) -> i32;
    fn RegCloseKey(key: Hkey) -> i32;
    fn RegQueryValueExW(
        key: Hkey,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegEnumKeyExW(
        key: Hkey,
        index: u32,
        name: *mut u16,
        name_size: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_size: *mut u32,
        written: *mut i64,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemFirmwareTable(provider: u32, table_id: u32, buffer: *mut u8, size: u32) -> u32;
    fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    fn GetLogicalDriveStringsW(size: u32, buffer: *mut u16) -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetDiskFreeSpaceExW(
        directory: *const u16,
        free_to_caller: *mut u64,
        total: *mut u64,
        free: *mut u64,
    ) -> i32;
    fn GetVolumeInformationW(
        root: *const u16,
        name: *mut u16,
        name_size: u32,
        serial: *mut u32,
        component_length: *mut u32,
        flags: *mut u32,
        filesystem: *mut u16,
        filesystem_size: u32,
    ) -> i32;
    fn GetTickCount64() -> u64;
    fn GetSystemTimeAsFileTime(time: *mut FileTime);
    fn GetNativeSystemInfo(info: *mut SystemInfo);
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
    fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn GetProcessTimes(
        process: isize,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn QueryFullProcessImageNameW(
        process: isize,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn GetTimeZoneInformation(info: *mut TimeZoneInformation) -> u32;
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetAdaptersAddresses(
        family: u32,
        flags: u32,
        reserved: *mut c_void,
        addresses: *mut IpAdapterAddresses,
        size: *mut u32,
    ) -> u32;
}

#[link(name = "netapi32")]
extern "system" {
    fn NetUserEnum(
        server: *const u16,
        level: u32,
        filter: u32,
        buffer: *mut *mut u8,
        prefmaxlen: u32,
        entries: *mut u32,
        total: *mut u32,
        handle: *mut u32,
    ) -> u32;
    fn NetApiBufferFree(buffer: *mut c_void) -> u32;
    fn NetLocalGroupEnum(
        server: *const u16,
        level: u32,
        buffer: *mut *mut u8,
        prefmaxlen: u32,
        entries: *mut u32,
        total: *mut u32,
        // PDWORD_PTR is pointer-sized, unlike NetUserEnum's PDWORD handle.
        handle: *mut usize,
    ) -> u32;
    fn NetLocalGroupGetMembers(
        server: *const u16,
        group: *const u16,
        level: u32,
        buffer: *mut *mut u8,
        prefmaxlen: u32,
        entries: *mut u32,
        total: *mut u32,
        // PDWORD_PTR is pointer-sized, unlike NetUserEnum's PDWORD handle.
        handle: *mut usize,
    ) -> u32;
}

#[repr(C)]
struct MemoryStatusEx {
    length: u32,
    memory_load: u32,
    total_physical: u64,
    available_physical: u64,
    total_page_file: u64,
    available_page_file: u64,
    total_virtual: u64,
    available_virtual: u64,
    available_extended_virtual: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn as_u64(self) -> u64 {
        (u64::from(self.high) << 32) | u64::from(self.low)
    }
}

#[repr(C)]
#[derive(Default)]
struct SystemInfo {
    processor_architecture: u16,
    reserved: u16,
    page_size: u32,
    minimum_application_address: usize,
    maximum_application_address: usize,
    active_processor_mask: usize,
    number_of_processors: u32,
    processor_type: u32,
    allocation_granularity: u32,
    processor_level: u16,
    processor_revision: u16,
}

#[repr(C)]
struct ProcessEntry32W {
    size: u32,
    usage: u32,
    process_id: u32,
    default_heap_id: usize,
    module_id: u32,
    threads: u32,
    parent_process_id: u32,
    priority: i32,
    flags: u32,
    executable: [u16; 260],
}

#[repr(C)]
struct TimeZoneInformation {
    bias: i32,
    standard_name: [u16; 32],
    standard_date: [u16; 8],
    standard_bias: i32,
    daylight_name: [u16; 32],
    daylight_date: [u16; 8],
    daylight_bias: i32,
}

#[repr(C)]
struct SocketAddress {
    sockaddr: *mut u8,
    length: i32,
}

#[repr(C)]
struct IpAdapterUnicastAddress {
    length: u32,
    flags: u32,
    next: *mut IpAdapterUnicastAddress,
    address: SocketAddress,
    prefix_origin: u32,
    suffix_origin: u32,
    dad_state: u32,
    valid_lifetime: u32,
    preferred_lifetime: u32,
    lease_lifetime: u32,
    prefix_length: u8,
}

/// IP_ADAPTER_ADDRESSES, declared only as far as the fields that are read.
///
/// The layout has to match exactly up to the last field used, because every
/// offset after a wrong one is garbage that still reads as a plausible number.
/// OperStatus follows IfType - not TunnelType - and the tail of the real
/// structure has grown across Windows versions, so nothing beyond OperStatus is
/// declared here rather than declared wrongly.
#[repr(C)]
struct IpAdapterAddresses {
    length: u32,
    index: u32,
    next: *mut IpAdapterAddresses,
    adapter_name: *mut u8,
    first_unicast: *mut IpAdapterUnicastAddress,
    first_anycast: *mut c_void,
    first_multicast: *mut c_void,
    first_dns_server: *mut c_void,
    dns_suffix: *mut u16,
    description: *mut u16,
    friendly_name: *mut u16,
    physical_address: [u8; 8],
    physical_address_length: u32,
    /// Present for layout: the DNS and DHCP flags are not reported.
    #[allow(dead_code)]
    flags: u32,
    mtu: u32,
    interface_type: u32,
    /// IF_OPER_STATUS: 1 is IfOperStatusUp.
    oper_status: u32,
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Takes the first `count` elements, clamped to what the buffer actually holds.
///
/// Win32 reports lengths in an out-parameter or a return value, and a caller that
/// slices with one of those directly is trusting it. Most of the time that trust
/// holds, but not always: GetLogicalDriveStringsW returns the *required* size when
/// the buffer was too small, which is larger than the buffer, and a driver is free
/// to report a hardware address longer than the fixed field it was written into.
/// Slicing past the end panics, and until the release profile stopped aborting on
/// panic that took the whole agent down - one adapter with an unusual address
/// length and the host collected nothing at all. Clamping loses a truncated value
/// in the rare wrong case and keeps every other collector alive.
fn take<T>(buffer: &[T], count: usize) -> &[T] {
    &buffer[..count.min(buffer.len())]
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..end])
        .to_string_lossy()
        .into_owned()
}

unsafe fn from_wide_ptr(pointer: *const u16) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let mut length = 0usize;
    while *pointer.add(length) != 0 {
        length += 1;
        if length > 32_768 {
            break;
        }
    }
    let slice = std::slice::from_raw_parts(pointer, length);
    Some(OsString::from_wide(slice).to_string_lossy().into_owned())
}

/// A registry view. 32-bit installers write under WOW6432Node, and an inventory
/// that reads only the native view misses a large share of installed software on
/// a 64-bit host.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegistryView {
    Native,
    Wow6432,
}

impl RegistryView {
    fn flag(self) -> u32 {
        match self {
            RegistryView::Native => KEY_WOW64_64KEY,
            RegistryView::Wow6432 => KEY_WOW64_32KEY,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RegistryView::Native => "native",
            RegistryView::Wow6432 => "wow6432",
        }
    }
}

struct Key(Hkey);

impl Key {
    fn open(root: Hkey, path: &str, view: RegistryView) -> Option<Self> {
        let mut handle: Hkey = 0;
        let status = unsafe {
            RegOpenKeyExW(
                root,
                wide(path).as_ptr(),
                0,
                KEY_READ | view.flag(),
                &mut handle,
            )
        };
        (status == ERROR_SUCCESS).then_some(Key(handle))
    }

    fn value(&self, name: &str) -> Option<RegistryValue> {
        let name = wide(name);
        let mut kind = 0u32;
        let mut size = 0u32;
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                buffer.as_mut_ptr(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        buffer.truncate(size as usize);
        match kind {
            REG_SZ | REG_EXPAND_SZ => {
                let wide: Vec<u16> = buffer
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                let text = from_wide(&wide);
                (!text.trim().is_empty()).then(|| RegistryValue::Text(text.trim().to_string()))
            }
            REG_DWORD if buffer.len() >= 4 => {
                Some(RegistryValue::Number(u64::from(u32::from_le_bytes([
                    buffer[0], buffer[1], buffer[2], buffer[3],
                ]))))
            }
            REG_QWORD if buffer.len() >= 8 => {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&buffer[..8]);
                Some(RegistryValue::Number(u64::from_le_bytes(bytes)))
            }
            _ => None,
        }
    }

    fn subkeys(&self) -> Vec<String> {
        let mut result = Vec::new();
        let mut index = 0u32;
        loop {
            let mut name = [0u16; 256];
            let mut length = name.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    name.as_mut_ptr(),
                    &mut length,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            result.push(from_wide(take(&name, length as usize)));
            index += 1;
            if index > 20_000 {
                break;
            }
        }
        result
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

#[derive(Debug, Clone)]
pub enum RegistryValue {
    Text(String),
    Number(u64),
}

impl RegistryValue {
    pub fn text(self) -> Option<String> {
        match self {
            RegistryValue::Text(value) => Some(value),
            RegistryValue::Number(value) => Some(value.to_string()),
        }
    }

    pub fn number(self) -> Option<u64> {
        match self {
            RegistryValue::Number(value) => Some(value),
            RegistryValue::Text(value) => value.trim().parse().ok(),
        }
    }
}

pub fn registry_string(root: isize, path: &str, name: &str) -> Option<String> {
    Key::open(root, path, RegistryView::Native)?
        .value(name)?
        .text()
}

pub fn registry_number(root: isize, path: &str, name: &str) -> Option<u64> {
    Key::open(root, path, RegistryView::Native)?
        .value(name)?
        .number()
}

/// Every value under one key, for the collectors that report a whole record.
pub fn registry_values(
    root: isize,
    path: &str,
    view: RegistryView,
    names: &[&str],
) -> Option<BTreeMap<String, RegistryValue>> {
    let key = Key::open(root, path, view)?;
    let mut result = BTreeMap::new();
    for name in names {
        if let Some(value) = key.value(name) {
            result.insert((*name).to_string(), value);
        }
    }
    Some(result)
}

pub fn registry_subkeys(root: isize, path: &str, view: RegistryView) -> Vec<String> {
    Key::open(root, path, view)
        .map(|key| key.subkeys())
        .unwrap_or_default()
}

/// The firmware UUID from SMBIOS structure type 1, the same value WMI reports as
/// Win32_ComputerSystemProduct.UUID, read without starting a COM apartment.
pub fn smbios_system_uuid() -> Option<String> {
    const RSMB: u32 = 0x5253_4D42; // 'RSMB'
    let size = unsafe { GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0) };
    if size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    let written = unsafe { GetSystemFirmwareTable(RSMB, 0, buffer.as_mut_ptr(), size) };
    if written == 0 || written > size {
        return None;
    }
    buffer.truncate(written as usize);
    // RawSMBIOSData: 4 bytes of version and flags, then Length, then the table.
    if buffer.len() < 8 {
        return None;
    }
    crate::smbios::system_uuid(&buffer[8..])
}

pub struct Memory {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub total_page_file_bytes: u64,
    pub available_page_file_bytes: u64,
    pub load_percent: u32,
}

pub fn memory() -> Option<Memory> {
    let mut status = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_physical: 0,
        available_physical: 0,
        total_page_file: 0,
        available_page_file: 0,
        total_virtual: 0,
        available_virtual: 0,
        available_extended_virtual: 0,
    };
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    Some(Memory {
        total_bytes: status.total_physical,
        available_bytes: status.available_physical,
        total_page_file_bytes: status.total_page_file,
        available_page_file_bytes: status.available_page_file,
        load_percent: status.memory_load,
    })
}

pub struct Processor {
    pub logical_processors: u32,
    pub architecture: &'static str,
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub megahertz: Option<u64>,
}

pub fn processor() -> Processor {
    let mut info = SystemInfo::default();
    unsafe { GetNativeSystemInfo(&mut info) };
    let architecture = match info.processor_architecture {
        9 => "x86_64",
        12 => "aarch64",
        0 => "x86",
        5 => "arm",
        _ => "unknown",
    };
    const CPU_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
    Processor {
        logical_processors: info.number_of_processors.max(1),
        architecture,
        name: registry_string(HKEY_LOCAL_MACHINE, CPU_KEY, "ProcessorNameString"),
        vendor: registry_string(HKEY_LOCAL_MACHINE, CPU_KEY, "VendorIdentifier"),
        megahertz: registry_number(HKEY_LOCAL_MACHINE, CPU_KEY, "~MHz"),
    }
}

pub struct Volume {
    pub root: String,
    pub drive_type: &'static str,
    pub filesystem: Option<String>,
    pub label: Option<String>,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub read_only: bool,
}

pub fn volumes() -> Vec<Volume> {
    let mut buffer = [0u16; 512];
    let written = unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) };
    if written == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    for root in take(&buffer, written as usize).split(|&c| c == 0) {
        if root.is_empty() {
            continue;
        }
        let root_text = from_wide(root);
        let root_wide = wide(&root_text);
        let drive_type = match unsafe { GetDriveTypeW(root_wide.as_ptr()) } {
            2 => "removable",
            3 => "fixed",
            4 => "network",
            5 => "optical",
            6 => "ram_disk",
            _ => "unknown",
        };
        // A removable or optical drive with no media answers with an error, and
        // reporting it as a zero-byte volume would look like a full disk.
        let mut total = 0u64;
        let mut free = 0u64;
        let mut free_to_caller = 0u64;
        // A network drive is someone else's storage, and asking for its size or its
        // label means a round trip that waits out the SMB timeout when the share is
        // gone. That parks the collector, and a parked collector used to stop the
        // whole cycle. It is still reported, just not measured.
        let remote = drive_type == "network";
        let sized = !remote
            && unsafe {
                GetDiskFreeSpaceExW(
                    root_wide.as_ptr(),
                    &mut free_to_caller,
                    &mut total,
                    &mut free,
                )
            } != 0;
        if !sized && drive_type != "fixed" && !remote {
            continue;
        }
        let mut label = [0u16; 256];
        let mut filesystem = [0u16; 64];
        let mut flags = 0u32;
        let described = !remote
            && unsafe {
                GetVolumeInformationW(
                    root_wide.as_ptr(),
                    label.as_mut_ptr(),
                    label.len() as u32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut flags,
                    filesystem.as_mut_ptr(),
                    filesystem.len() as u32,
                )
            } != 0;
        result.push(Volume {
            root: root_text,
            drive_type,
            filesystem: described
                .then(|| from_wide(&filesystem))
                .filter(|v| !v.is_empty()),
            label: described
                .then(|| from_wide(&label))
                .filter(|v| !v.is_empty()),
            total_bytes: total,
            free_bytes: free,
            // FILE_READ_ONLY_VOLUME
            read_only: described && flags & 0x0008_0000 != 0,
        });
    }
    result
}

pub struct Adapter {
    pub name: String,
    pub description: Option<String>,
    pub mac_address: Option<String>,
    pub addresses: Vec<String>,
    pub mtu: u32,
    pub interface_type: u32,
    pub operational: bool,
    pub dns_suffix: Option<String>,
}

pub fn adapters() -> Vec<Adapter> {
    const AF_UNSPEC: u32 = 0;
    // Skip what an inventory does not report, and ask for the friendly name.
    const SKIP: u32 = 0x0001 | 0x0002 | 0x0004 | 0x0008; // unicast kept, rest skipped
    let mut size = 16 * 1024u32;
    let mut buffer = vec![0u8; size as usize];
    let mut status = unsafe {
        GetAdaptersAddresses(
            AF_UNSPEC,
            SKIP & !0x0001,
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut IpAdapterAddresses,
            &mut size,
        )
    };
    if status == 111 {
        // ERROR_BUFFER_OVERFLOW: retry once with the size it asked for.
        buffer = vec![0u8; size as usize];
        status = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC,
                SKIP & !0x0001,
                std::ptr::null_mut(),
                buffer.as_mut_ptr() as *mut IpAdapterAddresses,
                &mut size,
            )
        };
    }
    if status != 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut current = buffer.as_ptr() as *const IpAdapterAddresses;
    unsafe {
        while !current.is_null() {
            let adapter = &*current;
            let name = from_wide_ptr(adapter.friendly_name)
                .or_else(|| from_wide_ptr(adapter.description))
                .unwrap_or_else(|| "unknown".to_string());
            let mac = (adapter.physical_address_length > 0).then(|| {
                take(
                    &adapter.physical_address,
                    adapter.physical_address_length as usize,
                )
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":")
            });
            let mut addresses = Vec::new();
            let mut unicast = adapter.first_unicast;
            while !unicast.is_null() {
                let entry = &*unicast;
                if let Some(text) = socket_address_text(&entry.address, entry.prefix_length) {
                    addresses.push(text);
                }
                unicast = entry.next;
            }
            result.push(Adapter {
                name,
                description: from_wide_ptr(adapter.description),
                mac_address: mac,
                addresses,
                mtu: adapter.mtu,
                interface_type: adapter.interface_type,
                // A disconnected adapter still appears with its configuration,
                // and reporting it as up would make a dead link look healthy.
                operational: adapter.oper_status == IF_OPER_STATUS_UP,
                dns_suffix: from_wide_ptr(adapter.dns_suffix).filter(|v| !v.is_empty()),
            });
            current = adapter.next;
        }
    }
    result
}

unsafe fn socket_address_text(address: &SocketAddress, prefix: u8) -> Option<String> {
    if address.sockaddr.is_null() || address.length < 2 {
        return None;
    }
    let family = u16::from_le_bytes([*address.sockaddr, *address.sockaddr.add(1)]);
    match family {
        2 if address.length >= 8 => {
            let octets = std::slice::from_raw_parts(address.sockaddr.add(4), 4);
            Some(format!(
                "{}.{}.{}.{}/{}",
                octets[0], octets[1], octets[2], octets[3], prefix
            ))
        }
        23 if address.length >= 24 => {
            let bytes = std::slice::from_raw_parts(address.sockaddr.add(8), 16);
            let groups: Vec<String> = bytes
                .chunks_exact(2)
                .map(|pair| format!("{:x}", u16::from_be_bytes([pair[0], pair[1]])))
                .collect();
            Some(format!("{}/{}", groups.join(":"), prefix))
        }
        _ => None,
    }
}

pub struct Process {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub threads: u32,
    pub executable_path: Option<String>,
    /// Creation time as a Windows FILETIME, used as the identity discriminator
    /// so a recycled PID is never mistaken for the same process.
    pub created_filetime: u64,
}

pub fn processes(limit: usize) -> Vec<Process> {
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == -1 {
        return Vec::new();
    }
    let mut entry = ProcessEntry32W {
        size: std::mem::size_of::<ProcessEntry32W>() as u32,
        usage: 0,
        process_id: 0,
        default_heap_id: 0,
        module_id: 0,
        threads: 0,
        parent_process_id: 0,
        priority: 0,
        flags: 0,
        executable: [0; 260],
    };
    let mut result = Vec::new();
    unsafe {
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let mut created = FileTime::default();
                let mut path = None;
                // A limited-information handle is enough for the times and the
                // image path, and asks for no more rights than an inventory needs.
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.process_id);
                if process != 0 {
                    let mut exit = FileTime::default();
                    let mut kernel = FileTime::default();
                    let mut user = FileTime::default();
                    GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user);
                    let mut buffer = [0u16; 1024];
                    let mut length = buffer.len() as u32;
                    if QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) != 0
                    {
                        path = Some(from_wide(take(&buffer, length as usize)));
                    }
                    CloseHandle(process);
                }
                result.push(Process {
                    pid: entry.process_id,
                    parent_pid: entry.parent_process_id,
                    name: from_wide(&entry.executable),
                    threads: entry.threads,
                    executable_path: path,
                    created_filetime: created.as_u64(),
                });
                if result.len() >= limit || Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    result
}

/// Seconds since the machine booted, from the tick count rather than a computed
/// difference of clocks that daylight saving can move.
pub fn uptime_seconds() -> u64 {
    (unsafe { GetTickCount64() }) / 1000
}

pub fn boot_time_unix() -> u64 {
    let mut now = FileTime::default();
    unsafe { GetSystemTimeAsFileTime(&mut now) };
    // FILETIME counts 100ns intervals since 1601-01-01.
    let unix_now = now.as_u64() / 10_000_000;
    let epoch_offset = 11_644_473_600u64;
    unix_now
        .saturating_sub(epoch_offset)
        .saturating_sub(uptime_seconds())
}

pub fn timezone_name() -> Option<String> {
    let mut info = TimeZoneInformation {
        bias: 0,
        standard_name: [0; 32],
        standard_date: [0; 8],
        standard_bias: 0,
        daylight_name: [0; 32],
        daylight_date: [0; 8],
        daylight_bias: 0,
    };
    let result = unsafe { GetTimeZoneInformation(&mut info) };
    if result == u32::MAX {
        return None;
    }
    // 2 == TIME_ZONE_ID_DAYLIGHT
    let name = if result == 2 {
        from_wide(&info.daylight_name)
    } else {
        from_wide(&info.standard_name)
    };
    (!name.is_empty()).then_some(name)
}

pub struct LocalUser {
    pub name: String,
    /// The account's relative identifier. Renaming an account keeps it, so it is
    /// the closest thing Windows has to a stable uid.
    pub rid: u32,
    pub full_name: Option<String>,
    pub comment: Option<String>,
    pub flags: u32,
    pub groups: Vec<String>,
}

/// USER_INFO_3. Level 3 rather than 2 because only level 3 carries the RID, and
/// an account record keyed on the name alone loses its identity on a rename.
#[repr(C)]
struct UserInfo3 {
    name: *mut u16,
    password: *mut u16,
    password_age: u32,
    priv_level: u32,
    home_dir: *mut u16,
    comment: *mut u16,
    flags: u32,
    script_path: *mut u16,
    auth_flags: u32,
    full_name: *mut u16,
    user_comment: *mut u16,
    parms: *mut u16,
    workstations: *mut u16,
    last_logon: u32,
    last_logoff: u32,
    account_expires: u32,
    max_storage: u32,
    units_per_week: u32,
    logon_hours: *mut u8,
    bad_pw_count: u32,
    num_logons: u32,
    logon_server: *mut u16,
    country_code: u32,
    code_page: u32,
    user_id: u32,
    primary_group_id: u32,
    profile: *mut u16,
    home_dir_drive: *mut u16,
    password_expired: u32,
}

/// USER_INFO_3 is thirty fields long and NetUserEnum hands back a buffer this
/// process only reinterprets, so a single field in the wrong place does not fail
/// to compile - it silently reads a pointer out of the middle of another field
/// and dereferences it. That is an access violation, which unwinding does not
/// catch and which leaves nothing in the log.
///
/// Wine stubs NetUserEnum, so `scripts/test-windows-wine.sh` cannot exercise this
/// path at all; it is the one collector with no execution behind it. These are
/// the offsets from lmaccess.h under the 64-bit ABI, where the padding lands at
/// 44, 60 and 116. Checked at compile time because there is nothing else to check
/// it with.
const _: () = {
    use std::mem::{align_of, offset_of, size_of};
    assert!(align_of::<UserInfo3>() == 8);
    assert!(size_of::<UserInfo3>() == 184);
    assert!(offset_of!(UserInfo3, name) == 0);
    assert!(offset_of!(UserInfo3, comment) == 32);
    assert!(offset_of!(UserInfo3, flags) == 40);
    assert!(offset_of!(UserInfo3, script_path) == 48);
    assert!(offset_of!(UserInfo3, full_name) == 64);
    assert!(offset_of!(UserInfo3, logon_hours) == 120);
    assert!(offset_of!(UserInfo3, logon_server) == 136);
    assert!(offset_of!(UserInfo3, user_id) == 152);
    assert!(offset_of!(UserInfo3, profile) == 160);
    assert!(offset_of!(UserInfo3, home_dir_drive) == 168);
    assert!(offset_of!(UserInfo3, password_expired) == 176);
};

#[repr(C)]
struct LocalGroupInfo0 {
    name: *mut u16,
}

#[repr(C)]
struct LocalGroupMembersInfo3 {
    domain_and_name: *mut u16,
}

pub fn local_users() -> Vec<LocalUser> {
    const FILTER_NORMAL_ACCOUNT: u32 = 0x0002;
    const MAX_PREFERRED_LENGTH: u32 = u32::MAX;
    let mut buffer: *mut u8 = std::ptr::null_mut();
    let mut entries = 0u32;
    let mut total = 0u32;
    // NetUserEnum uses PDWORD, which remains 32-bit on 64-bit Windows.
    let mut handle = 0u32;
    let status = unsafe {
        NetUserEnum(
            std::ptr::null(),
            3,
            FILTER_NORMAL_ACCOUNT,
            &mut buffer,
            MAX_PREFERRED_LENGTH,
            &mut entries,
            &mut total,
            &mut handle,
        )
    };
    if status != 0 || buffer.is_null() {
        return Vec::new();
    }
    let memberships = local_group_memberships();
    let mut result = Vec::new();
    unsafe {
        let items = std::slice::from_raw_parts(buffer as *const UserInfo3, entries as usize);
        for item in items {
            let Some(name) = from_wide_ptr(item.name) else {
                continue;
            };
            let groups = memberships
                .get(&name.to_lowercase())
                .cloned()
                .unwrap_or_default();
            result.push(LocalUser {
                rid: item.user_id,
                full_name: from_wide_ptr(item.full_name).filter(|v| !v.is_empty()),
                comment: from_wide_ptr(item.comment).filter(|v| !v.is_empty()),
                flags: item.flags,
                groups,
                name,
            });
        }
        NetApiBufferFree(buffer as *mut c_void);
    }
    result
}

/// Which local groups each account belongs to. Group membership is what decides
/// whether an account is an administrator, so an inventory that omits it cannot
/// answer the question anyone asks of it.
/// The local groups whose membership decides whether an account is privileged.
/// A localised Windows installation names them differently; those are simply not
/// queried rather than guessed at, so the field is absent instead of wrong.
const PRIVILEGED_GROUPS: &[&str] = &[
    "Administrators",
    "Remote Desktop Users",
    "Backup Operators",
    "Power Users",
    "Remote Management Users",
];

fn local_group_memberships() -> BTreeMap<String, Vec<String>> {
    const MAX_PREFERRED_LENGTH: u32 = u32::MAX;
    let mut memberships: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut group_buffer: *mut u8 = std::ptr::null_mut();
    let mut group_count = 0u32;
    let mut total = 0u32;
    // NetLocalGroupEnum uses PDWORD_PTR. Keeping this pointer-sized is an ABI
    // requirement on 64-bit Windows: a u32 lets Netapi32 write eight bytes into
    // a four-byte stack slot and corrupt the account collector's stack.
    let mut handle = 0usize;
    let status = unsafe {
        NetLocalGroupEnum(
            std::ptr::null(),
            0,
            &mut group_buffer,
            MAX_PREFERRED_LENGTH,
            &mut group_count,
            &mut total,
            &mut handle,
        )
    };
    if status != 0 || group_buffer.is_null() {
        return memberships;
    }
    unsafe {
        let groups = std::slice::from_raw_parts(
            group_buffer as *const LocalGroupInfo0,
            group_count as usize,
        );
        for group in groups {
            let Some(group_name) = from_wide_ptr(group.name) else {
                continue;
            };
            // Level 3 translates every member SID to a DOMAIN\name, and on a
            // domain-joined host that is a call to a domain controller. Doing it
            // for all of the built-in groups multiplies the exposure to a slow or
            // unreachable DC for no gain: what an inventory is asked is which
            // accounts are privileged.
            if !PRIVILEGED_GROUPS
                .iter()
                .any(|candidate| group_name.eq_ignore_ascii_case(candidate))
            {
                continue;
            }
            let mut member_buffer: *mut u8 = std::ptr::null_mut();
            let mut member_count = 0u32;
            let mut member_total = 0u32;
            // NetLocalGroupGetMembers also uses PDWORD_PTR (not PDWORD).
            let mut member_handle = 0usize;
            let status = NetLocalGroupGetMembers(
                std::ptr::null(),
                wide(&group_name).as_ptr(),
                3,
                &mut member_buffer,
                MAX_PREFERRED_LENGTH,
                &mut member_count,
                &mut member_total,
                &mut member_handle,
            );
            if status != 0 || member_buffer.is_null() {
                continue;
            }
            let members = std::slice::from_raw_parts(
                member_buffer as *const LocalGroupMembersInfo3,
                member_count as usize,
            );
            for member in members {
                if let Some(qualified) = from_wide_ptr(member.domain_and_name) {
                    let account = qualified
                        .rsplit('\\')
                        .next()
                        .unwrap_or(&qualified)
                        .to_lowercase();
                    memberships
                        .entry(account)
                        .or_default()
                        .push(group_name.clone());
                }
            }
            NetApiBufferFree(member_buffer as *mut c_void);
        }
        NetApiBufferFree(group_buffer as *mut c_void);
    }
    for groups in memberships.values_mut() {
        groups.sort();
        groups.dedup();
    }
    memberships
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> isize;
    fn CloseServiceHandle(handle: isize) -> i32;
    fn EnumServicesStatusExW(
        manager: isize,
        info_level: u32,
        service_type: u32,
        service_state: u32,
        services: *mut u8,
        size: u32,
        needed: *mut u32,
        returned: *mut u32,
        resume: *mut u32,
        group: *const u16,
    ) -> i32;
}

#[repr(C)]
struct ServiceStatusProcess {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    check_point: u32,
    wait_hint: u32,
    process_id: u32,
    service_flags: u32,
}

#[repr(C)]
struct EnumServiceStatusProcessW {
    service_name: *mut u16,
    display_name: *mut u16,
    status: ServiceStatusProcess,
}

pub struct Service {
    pub name: String,
    pub display_name: Option<String>,
    pub state: &'static str,
    pub service_type: &'static str,
    pub process_id: Option<u32>,
    /// From the registry: 2 automatic, 3 manual, 4 disabled.
    pub start_type: Option<&'static str>,
    pub run_as: Option<String>,
    pub image_path: Option<String>,
    pub delayed_auto_start: bool,
}

/// Enumerates services from the Service Control Manager, then fills in the
/// configuration the SCM enumeration does not carry - start type, the account it
/// runs as and the image path - from the service's own registry key. Both halves
/// matter: the state answers "is it running", the configuration answers "will it
/// come back after a reboot", and a great deal of Windows drift is the second.
pub fn services() -> Vec<Service> {
    const SC_MANAGER_ENUMERATE_SERVICE: u32 = 0x0004;
    const SC_ENUM_PROCESS_INFO: u32 = 0;
    const SERVICE_WIN32: u32 = 0x0000_0030;
    const SERVICE_DRIVER: u32 = 0x0000_000B;
    const SERVICE_STATE_ALL: u32 = 0x0000_0003;
    const ERROR_MORE_DATA_LOCAL: i32 = 234;

    let manager = unsafe {
        OpenSCManagerW(
            std::ptr::null(),
            std::ptr::null(),
            SC_MANAGER_ENUMERATE_SERVICE,
        )
    };
    if manager == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    // Roughly 250 services on a typical host, 56 bytes each plus their name and
    // display name, so this usually holds the whole answer in one call.
    let mut buffer = vec![0u8; 256 * 1024];
    let mut resume = 0u32;
    // A bound on the retries, so a driver that keeps asking for more room cannot
    // turn this into an unbounded loop.
    let mut attempts = 0u32;
    loop {
        let mut needed = 0u32;
        let mut returned = 0u32;
        let ok = unsafe {
            EnumServicesStatusExW(
                manager,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32 | SERVICE_DRIVER,
                SERVICE_STATE_ALL,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut needed,
                &mut returned,
                &mut resume,
                std::ptr::null(),
            )
        };
        // `!ok` is a bitwise complement, not a logical negation: for any i32 it
        // is non-zero, so the previous form said "more data" on every outcome and
        // an unexpected error fell through to a loop that never terminated - the
        // collector would hang, and a hung collector stops the whole cycle, so
        // nothing is ever queued or delivered.
        let failed = ok == 0;
        let needs_room = failed
            && unsafe { last_error() } == ERROR_MORE_DATA_LOCAL
            && needed as usize > buffer.len();
        if needs_room {
            // Grow to the size the SCM asked for and retry the same page.
            buffer = vec![0u8; needed as usize];
            attempts += 1;
            if attempts > 8 {
                break;
            }
            continue;
        }
        if failed {
            // Any other failure - and a "more data" answer that does not ask for
            // a larger buffer - ends the enumeration with what was collected.
            break;
        }
        // The buffer belongs to this process, not to the SCM, so `returned` and
        // the room actually available can disagree - and unlike a bad slice index
        // this one reads past the allocation instead of panicking, which unwinding
        // cannot catch and which yields a plausible-looking service entry built
        // from whatever followed in memory. Trust the smaller of the two.
        let capacity = buffer.len() / std::mem::size_of::<EnumServiceStatusProcessW>();
        if returned as usize > capacity {
            tracing::warn!(
                returned,
                capacity,
                "the service control manager reported more services than the buffer holds"
            );
        }
        unsafe {
            let items = std::slice::from_raw_parts(
                buffer.as_ptr() as *const EnumServiceStatusProcessW,
                (returned as usize).min(capacity),
            );
            for item in items {
                let Some(name) = from_wide_ptr(item.service_name) else {
                    continue;
                };
                let configuration = service_configuration(&name);
                result.push(Service {
                    display_name: from_wide_ptr(item.display_name),
                    state: match item.status.current_state {
                        1 => "stopped",
                        2 => "start_pending",
                        3 => "stop_pending",
                        4 => "running",
                        5 => "continue_pending",
                        6 => "pause_pending",
                        7 => "paused",
                        _ => "unknown",
                    },
                    service_type: if item.status.service_type & 0x0000_000B != 0 {
                        "driver"
                    } else if item.status.service_type & 0x0000_0010 != 0 {
                        "own_process"
                    } else if item.status.service_type & 0x0000_0020 != 0 {
                        "shared_process"
                    } else {
                        "other"
                    },
                    process_id: (item.status.process_id != 0).then_some(item.status.process_id),
                    start_type: configuration.0,
                    run_as: configuration.1,
                    image_path: configuration.2,
                    delayed_auto_start: configuration.3,
                    name,
                });
            }
        }
        if ok != 0 {
            break;
        }
    }
    unsafe {
        CloseServiceHandle(manager);
    }
    result
}

fn service_configuration(
    name: &str,
) -> (Option<&'static str>, Option<String>, Option<String>, bool) {
    let path = format!(r"SYSTEM\CurrentControlSet\Services\{name}");
    let start = registry_number(HKEY_LOCAL_MACHINE, &path, "Start").map(|value| match value {
        0 => "boot",
        1 => "system",
        2 => "automatic",
        3 => "manual",
        4 => "disabled",
        _ => "unknown",
    });
    (
        start,
        registry_string(HKEY_LOCAL_MACHINE, &path, "ObjectName"),
        registry_string(HKEY_LOCAL_MACHINE, &path, "ImagePath"),
        registry_number(HKEY_LOCAL_MACHINE, &path, "DelayedAutostart") == Some(1),
    )
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
}

unsafe fn last_error() -> i32 {
    GetLastError() as i32
}

#[link(name = "advapi32")]
extern "system" {
    fn GetNamedSecurityInfoW(
        object: *const u16,
        object_type: u32,
        security_info: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        descriptor: *mut *mut c_void,
    ) -> u32;
    fn GetEffectiveRightsFromAclW(
        acl: *const c_void,
        trustee: *const TrusteeW,
        access: *mut u32,
    ) -> u32;
    fn CreateWellKnownSid(kind: u32, domain: *const c_void, sid: *mut u8, size: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[repr(C)]
struct TrusteeW {
    multiple_trustee: *mut c_void,
    multiple_trustee_operation: u32,
    trustee_form: u32,
    trustee_type: u32,
    name: *mut u16,
}

/// Well-known accounts an inventory agent's files must not be readable by.
/// `WinWorldSid` is Everyone; `WinBuiltinUsersSid` is the local Users group,
/// which every interactive account belongs to.
const WIN_WORLD_SID: u32 = 1;
const WIN_BUILTIN_USERS_SID: u32 = 27;

/// Whether an ordinary user can read this path.
///
/// The Linux risk is a configuration the service *cannot* read. On Windows the
/// service runs as LocalSystem and can read anything, so the risk inverts: a file
/// created outside the installer's ACL - copied in by hand, or dropped in a
/// directory that inherits the permissive default on a volume root - is readable
/// by every logged-in user, and the configuration can hold a bearer token.
///
/// Returns None when the answer cannot be determined, which must not be reported
/// as either safe or unsafe.
pub fn readable_by_ordinary_users(path: &std::path::Path) -> Option<bool> {
    const SE_FILE_OBJECT: u32 = 1;
    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const FILE_READ_DATA: u32 = 0x0001;
    const GENERIC_READ: u32 = 0x8000_0000;

    let wide_path = wide(&path.to_string_lossy());
    let mut dacl: *mut c_void = std::ptr::null_mut();
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 || dacl.is_null() {
        if !descriptor.is_null() {
            unsafe { LocalFree(descriptor) };
        }
        return None;
    }
    let mut permitted = false;
    let mut known = false;
    for kind in [WIN_WORLD_SID, WIN_BUILTIN_USERS_SID] {
        let mut sid = vec![0u8; 68];
        let mut size = sid.len() as u32;
        if unsafe { CreateWellKnownSid(kind, std::ptr::null(), sid.as_mut_ptr(), &mut size) } == 0 {
            continue;
        }
        let trustee = TrusteeW {
            multiple_trustee: std::ptr::null_mut(),
            multiple_trustee_operation: 0,
            // TRUSTEE_IS_SID
            trustee_form: 0,
            // TRUSTEE_IS_UNKNOWN
            trustee_type: 0,
            name: sid.as_mut_ptr() as *mut u16,
        };
        let mut access = 0u32;
        if unsafe { GetEffectiveRightsFromAclW(dacl, &trustee, &mut access) } == 0 {
            known = true;
            if access & (FILE_READ_DATA | GENERIC_READ) != 0 {
                permitted = true;
            }
        }
    }
    unsafe { LocalFree(descriptor) };
    known.then_some(permitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The local-group APIs deliberately differ from NetUserEnum here:
    /// Microsoft declares their resume handle as PDWORD_PTR, so changing either
    /// declaration back to `*mut u32` corrupts the stack on 64-bit Windows.
    #[test]
    fn local_group_resume_handles_are_pointer_sized() {
        type GroupEnum = unsafe extern "system" fn(
            *const u16,
            u32,
            *mut *mut u8,
            u32,
            *mut u32,
            *mut u32,
            *mut usize,
        ) -> u32;
        type GroupMembers = unsafe extern "system" fn(
            *const u16,
            *const u16,
            u32,
            *mut *mut u8,
            u32,
            *mut u32,
            *mut u32,
            *mut usize,
        ) -> u32;

        let _: GroupEnum = NetLocalGroupEnum;
        let _: GroupMembers = NetLocalGroupGetMembers;
        assert_eq!(
            std::mem::size_of::<usize>(),
            std::mem::size_of::<*mut c_void>()
        );
    }

    /// Exercises the complete native path, including both local-group calls.
    /// The former ABI mismatch terminated this test with 0xC0000005 before an
    /// assertion could run on a 64-bit Windows host.
    #[test]
    fn local_accounts_can_be_enumerated_without_corrupting_memory() {
        let users = local_users();
        // A locked-down caller may legitimately receive no level-3 records.
        // The ABI regression is covered by the signature test above and, when
        // records are available, this call still exercises both native APIs.
        assert!(users.iter().all(|user| !user.name.trim().is_empty()));
    }
}
