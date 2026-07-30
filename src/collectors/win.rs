//! The Windows inventory collectors.
//!
//! Each one produces the same record categories as its Linux counterpart, so the
//! Server's classification rules, relationship inference and asset keys work
//! unchanged across a mixed estate. Where Windows has no equivalent of a Linux
//! field the field is absent rather than invented, and where Windows carries
//! something Linux does not - a service start type, a package publisher, an
//! account's group membership - it is reported, because those are the fields a
//! Windows estate is actually managed by.

use super::{record, Collector};
use crate::config::CollectorConfig;
use crate::model::AssetRecord;
use crate::platform;
use crate::windows_inventory::{edition_name, interface_type_name, percent, UninstallEntry};
use crate::windows_sys as sys;
use anyhow::Result;
use serde_json::json;
use std::sync::Arc;

pub fn configured(config: &CollectorConfig) -> Vec<Arc<dyn Collector>> {
    let mut result: Vec<Arc<dyn Collector>> = Vec::new();
    macro_rules! enabled {
        ($field:ident, $collector:expr) => {
            if config.$field {
                result.push(Arc::new($collector));
            }
        };
    }
    enabled!(os, SystemCollector);
    enabled!(cpu, CpuCollector);
    enabled!(memory, MemoryCollector);
    enabled!(disk, VolumeCollector);
    enabled!(network, AdapterCollector);
    enabled!(
        process,
        ProcessCollector {
            max_processes: config.max_processes,
        }
    );
    enabled!(packages, InstalledSoftwareCollector);
    enabled!(services, ServiceCollector);
    enabled!(accounts, AccountCollector);
    enabled!(containers, ContainerCollector);
    result
}

/// Windows edition, build and install identity.
///
/// `CurrentBuild` is the value that actually distinguishes Windows releases:
/// `CurrentVersion` has read "6.3" since Windows 8.1 and `ProductName` still says
/// "Windows 10" on Windows 11 hosts, so an inventory that trusts either reports
/// the wrong operating system. The display version (23H2) and the UBR - the
/// fourth part of the build number that patching moves - are what a patch level
/// is judged by.
pub struct SystemCollector;

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

impl Collector for SystemCollector {
    fn name(&self) -> &'static str {
        "os"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        let read =
            |name: &str| sys::registry_string(sys::HKEY_LOCAL_MACHINE, CURRENT_VERSION, name);
        let build = read("CurrentBuildNumber").or_else(|| read("CurrentBuild"));
        let ubr = sys::registry_number(sys::HKEY_LOCAL_MACHINE, CURRENT_VERSION, "UBR");
        let display_version = read("DisplayVersion").or_else(|| read("ReleaseId"));
        let product = read("ProductName");
        let full_build = match (&build, ubr) {
            (Some(build), Some(ubr)) => Some(format!("{build}.{ubr}")),
            (Some(build), None) => Some(build.clone()),
            _ => None,
        };
        let identifiers = platform::machine_identifiers();
        let payload = json!({
            "hostname": platform::hostname(),
            "os_family": "windows",
            "os_name": edition_name(product.as_deref(), build.as_deref()),
            "os_product_name": product,
            "os_version": display_version,
            "os_build": full_build,
            "kernel_version": build,
            "architecture": sys::processor().architecture,
            "install_type": read("InstallationType"),
            "edition_id": read("EditionID"),
            "boot_time": sys::boot_time_unix(),
            "uptime_seconds": sys::uptime_seconds(),
            "timezone": sys::timezone_name(),
            "machine_id": identifiers.machine_id,
            "firmware_uuid": identifiers.firmware_uuid,
            "domain_role": domain_membership(),
        });
        Ok(vec![record("system", "registry", collected_at, payload)])
    }
}

/// Whether the host is domain-joined, which decides how its accounts and policy
/// are managed and is the first thing asked of a Windows asset.
fn domain_membership() -> serde_json::Value {
    let domain = sys::registry_string(
        sys::HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters",
        "Domain",
    )
    .filter(|value| !value.trim().is_empty());
    match domain {
        Some(domain) => json!({"joined": true, "domain": domain}),
        None => json!({"joined": false}),
    }
}

pub struct CpuCollector;

impl Collector for CpuCollector {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        let processor = sys::processor();
        let payload = json!({
            "logical_processors": processor.logical_processors,
            "model": processor.name,
            "vendor": processor.vendor,
            "megahertz": processor.megahertz,
            "architecture": processor.architecture,
        });
        Ok(vec![record(
            "hardware.cpu",
            "registry",
            collected_at,
            payload,
        )])
    }
}

pub struct MemoryCollector;

impl Collector for MemoryCollector {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        let memory = sys::memory().ok_or_else(|| anyhow::anyhow!("GlobalMemoryStatusEx failed"))?;
        let payload = json!({
            "total_bytes": memory.total_bytes,
            "available_bytes": memory.available_bytes,
            "used_percent": memory.load_percent,
            "page_file_total_bytes": memory.total_page_file_bytes,
            "page_file_available_bytes": memory.available_page_file_bytes,
        });
        Ok(vec![record(
            "hardware.memory",
            "GlobalMemoryStatusEx",
            collected_at,
            payload,
        )])
    }
}

pub struct VolumeCollector;

impl Collector for VolumeCollector {
    fn name(&self) -> &'static str {
        "disk"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        Ok(sys::volumes()
            .into_iter()
            .map(|volume| {
                let used = volume.total_bytes.saturating_sub(volume.free_bytes);
                let payload = json!({
                    // The same field name the Linux collector uses, so one
                    // filesystem asset key covers both platforms.
                    "mountpoint": volume.root,
                    "device": volume.root,
                    "filesystem": volume.filesystem,
                    "label": volume.label,
                    "drive_type": volume.drive_type,
                    "total_bytes": volume.total_bytes,
                    "free_bytes": volume.free_bytes,
                    "used_bytes": used,
                    "used_percent": percent(used, volume.total_bytes),
                    "read_only": volume.read_only,
                });
                record("hardware.filesystem", "Win32 volume", collected_at, payload)
            })
            .collect())
    }
}

pub struct AdapterCollector;

impl Collector for AdapterCollector {
    fn name(&self) -> &'static str {
        "network"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        let adapters = sys::adapters();
        let mut records: Vec<AssetRecord> = adapters
            .iter()
            .map(|adapter| {
                let payload = json!({
                    "name": adapter.name,
                    "description": adapter.description,
                    "mac_address": adapter.mac_address,
                    "addresses": adapter.addresses,
                    "mtu": adapter.mtu,
                    "interface_type": interface_type_name(adapter.interface_type),
                    "dns_suffix": adapter.dns_suffix,
                });
                record(
                    "network.interface",
                    "GetAdaptersAddresses",
                    collected_at,
                    payload,
                )
            })
            .collect();
        // One configuration record per host, matching the Linux collector, so a
        // host's DNS and search domain land in the same place on both platforms.
        let search_domains: Vec<String> = adapters
            .iter()
            .filter_map(|adapter| adapter.dns_suffix.clone())
            .collect();
        records.push(record(
            "network.configuration",
            "GetAdaptersAddresses",
            collected_at,
            json!({
                "hostname": platform::hostname(),
                "search_domains": deduplicate(search_domains),
                "adapter_count": adapters.len(),
            }),
        ));
        Ok(records)
    }
}

fn deduplicate(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

pub struct ProcessCollector {
    pub max_processes: usize,
}

impl Collector for ProcessCollector {
    fn name(&self) -> &'static str {
        "process"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        Ok(sys::processes(self.max_processes)
            .into_iter()
            .map(|process| {
                let payload = json!({
                    "pid": process.pid,
                    "parent_pid": process.parent_pid,
                    "name": process.name,
                    "executable": process.executable_path,
                    "threads": process.threads,
                    // The Linux collector discriminates a recycled PID by the
                    // process start time in clock ticks; the creation FILETIME is
                    // the same idea and keeps one asset key rule for both.
                    "start_ticks": process.created_filetime,
                });
                record("process", "Toolhelp32", collected_at, payload)
            })
            .collect())
    }
}

/// Installed software from the Uninstall keys.
///
/// This is the authoritative Windows software inventory, and it has to be read
/// from three places: the 64-bit machine view, the 32-bit (WOW6432Node) machine
/// view, and each user's own hive for per-user installs. Reading only the first
/// is the usual mistake and silently omits every 32-bit product and everything
/// installed by a user - on a developer or office estate, most of the list.
///
/// Win32_Product (the WMI class) is deliberately not used: querying it triggers
/// an MSI self-repair consistency check on every installed package, which is slow
/// and has been known to change the machine it was only supposed to inventory.
pub struct InstalledSoftwareCollector;

const UNINSTALL_PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

impl Collector for InstalledSoftwareCollector {
    fn name(&self) -> &'static str {
        "packages"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        let mut records = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let machine_scopes = [
            (sys::RegistryView::Native, "machine"),
            (sys::RegistryView::Wow6432, "machine-x86"),
        ];
        for (view, scope) in machine_scopes {
            collect_uninstall_keys(
                sys::HKEY_LOCAL_MACHINE,
                UNINSTALL_PATH,
                view,
                scope,
                collected_at,
                &mut seen,
                &mut records,
            );
        }
        // Per-user installs live under each loaded profile in HKEY_USERS. A
        // service sees every loaded hive, which is what makes this readable at
        // all from a non-interactive context.
        for sid in sys::registry_subkeys(sys::HKEY_USERS, "", sys::RegistryView::Native) {
            if sid.ends_with("_Classes") || sid == ".DEFAULT" {
                continue;
            }
            let path = format!(r"{sid}\{UNINSTALL_PATH}");
            collect_uninstall_keys(
                sys::HKEY_USERS,
                &path,
                sys::RegistryView::Native,
                "user",
                collected_at,
                &mut seen,
                &mut records,
            );
        }
        Ok(records)
    }
}

fn collect_uninstall_keys(
    root: isize,
    path: &str,
    view: sys::RegistryView,
    scope: &str,
    collected_at: u64,
    seen: &mut std::collections::BTreeSet<String>,
    records: &mut Vec<AssetRecord>,
) {
    const FIELDS: &[&str] = &[
        "DisplayName",
        "DisplayVersion",
        "Publisher",
        "InstallDate",
        "InstallLocation",
        "EstimatedSize",
        "SystemComponent",
        "ReleaseType",
        "ParentKeyName",
        "WindowsInstaller",
    ];
    for key in sys::registry_subkeys(root, path, view) {
        let full = format!("{path}\\{key}");
        let Some(values) = sys::registry_values(root, &full, view, FIELDS) else {
            continue;
        };
        let text = |name: &str| values.get(name).cloned().and_then(|value| value.text());
        let number = |name: &str| values.get(name).cloned().and_then(|value| value.number());
        let Some(display_name) = text("DisplayName") else {
            continue;
        };
        let release_type = text("ReleaseType");
        let parent_key = text("ParentKeyName");
        if !crate::windows_inventory::is_reportable_product(&UninstallEntry {
            display_name: Some(&display_name),
            system_component: number("SystemComponent") == Some(1),
            release_type: release_type.as_deref(),
            parent_key_name: parent_key.as_deref(),
        }) {
            continue;
        }
        let version = text("DisplayVersion");
        let identity = format!(
            "{}|{}|{}",
            display_name.to_lowercase(),
            version.clone().unwrap_or_default().to_lowercase(),
            scope
        );
        if !seen.insert(identity) {
            continue;
        }
        let payload = json!({
            "manager": "windows",
            "name": display_name,
            "version": version,
            "publisher": text("Publisher"),
            "architecture": match view {
                sys::RegistryView::Native => "x64",
                sys::RegistryView::Wow6432 => "x86",
            },
            "scope": scope,
            "install_date": text("InstallDate"),
            "install_location": text("InstallLocation"),
            "estimated_size_kb": number("EstimatedSize"),
            "registry_key": key,
            "installer": if number("WindowsInstaller") == Some(1) { "msi" } else { "other" },
        });
        records.push(record(
            "software.package",
            "uninstall registry",
            collected_at,
            payload,
        ));
    }
}

pub struct ServiceCollector;

impl Collector for ServiceCollector {
    fn name(&self) -> &'static str {
        "services"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        Ok(sys::services()
            .into_iter()
            .map(|service| {
                let payload = json!({
                    "manager": "windows",
                    "name": service.name,
                    "display_name": service.display_name,
                    "state": service.state,
                    "active": service.state == "running",
                    // The start type is what says whether a stopped service is
                    // meant to be stopped, which "state" alone cannot answer.
                    "start_type": service.start_type,
                    "enabled": matches!(service.start_type, Some("automatic") | Some("boot") | Some("system")),
                    "delayed_auto_start": service.delayed_auto_start,
                    "service_type": service.service_type,
                    "run_as": service.run_as,
                    "image_path": service.image_path,
                    "pid": service.process_id,
                });
                record("service", "service control manager", collected_at, payload)
            })
            .collect())
    }
}

pub struct AccountCollector;

impl Collector for AccountCollector {
    fn name(&self) -> &'static str {
        "accounts"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        Ok(sys::local_users()
            .into_iter()
            .map(|user| {
                let payload = json!({
                    "name": user.name,
                    // The RID is the account's stable identity: renaming an
                    // account keeps it, and it is the closest thing Windows has
                    // to a uid.
                    "uid": user.rid,
                    "full_name": user.full_name,
                    "comment": user.comment,
                    // Group membership is what decides whether an account is
                    // privileged, which is the question asked of this record.
                    "groups": user.groups,
                    "administrator": user
                        .groups
                        .iter()
                        .any(|group| group.eq_ignore_ascii_case("Administrators")),
                    "disabled": user.flags & 0x0002 != 0,
                    "locked_out": user.flags & 0x0010 != 0,
                    "password_not_required": user.flags & 0x0020 != 0,
                    "password_never_expires": user.flags & 0x1_0000 != 0,
                });
                record("account.user", "NetUserEnum", collected_at, payload)
            })
            .collect())
    }
}

pub struct ContainerCollector;

impl Collector for ContainerCollector {
    fn name(&self) -> &'static str {
        "containers"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
        // Docker Desktop and the Windows container feature both present a named
        // pipe rather than a Unix socket, and the service registration is the
        // reliable signal that the runtime is installed rather than merely
        // downloaded.
        let runtimes: Vec<&str> = [
            ("docker", r"SYSTEM\CurrentControlSet\Services\docker"),
            (
                "com.docker.service",
                r"SYSTEM\CurrentControlSet\Services\com.docker.service",
            ),
            (
                "containerd",
                r"SYSTEM\CurrentControlSet\Services\containerd",
            ),
        ]
        .into_iter()
        .filter(|(_, key)| {
            sys::registry_string(sys::HKEY_LOCAL_MACHINE, key, "ImagePath").is_some()
        })
        .map(|(name, _)| name)
        .collect();
        let payload = json!({
            "runtimes": runtimes,
            "in_container": std::env::var_os("CONTAINER_SANDBOX_MOUNT_POINT").is_some(),
            "windows_containers_feature": sys::registry_string(
                sys::HKEY_LOCAL_MACHINE,
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ServerFeatures",
                "Containers",
            )
            .is_some(),
        });
        Ok(vec![record(
            "container.environment",
            "registry",
            collected_at,
            payload,
        )])
    }
}
