//! The judgement calls the Windows collectors make about what they read.
//!
//! Reading the registry is mechanical; deciding what a value *means* is not, and
//! those decisions are what make a Windows inventory usable or useless. They live
//! here, apart from the registry access, so they are tested on every platform
//! rather than only where they run.

/// Corrects the operating system name.
///
/// Every Windows 11 host reports `ProductName` as "Windows 10 …" - Microsoft never
/// updated the value, and a great deal of third-party inventory reports the whole
/// Windows 11 estate as Windows 10 because of it. Build 22000 is the first
/// Windows 11 build, so the build number is what the name is corrected from.
pub fn edition_name(product: Option<&str>, build: Option<&str>) -> String {
    let product = product.unwrap_or("Windows");
    let build_number: u32 = build.and_then(|value| value.parse().ok()).unwrap_or(0);
    if build_number >= 22_000 && product.contains("Windows 10") {
        return product.replace("Windows 10", "Windows 11");
    }
    product.to_string()
}

/// A share of a whole, or None when there is no whole. An empty optical drive
/// reports a zero-byte capacity, and "0% used" would read as a healthy volume.
///
/// The multiply is widened rather than saturated: saturating first and dividing
/// second turns a full volume into 1% once the byte count is large enough to clamp.
pub fn percent(part: u64, whole: u64) -> Option<u64> {
    (whole > 0).then(|| (u128::from(part) * 100 / u128::from(whole)) as u64)
}

/// IANA interface types, named. Reporting the number would push the translation
/// onto whoever reads the inventory.
pub fn interface_type_name(value: u32) -> &'static str {
    match value {
        6 => "ethernet",
        23 => "ppp",
        24 => "loopback",
        71 => "wireless",
        131 => "tunnel",
        237 => "ib",
        _ => "other",
    }
}

/// What an Uninstall registry entry carries that decides whether it is a product.
pub struct UninstallEntry<'a> {
    pub display_name: Option<&'a str>,
    pub system_component: bool,
    pub release_type: Option<&'a str>,
    pub parent_key_name: Option<&'a str>,
}

/// Whether an Uninstall entry is a product a person installed.
///
/// The Uninstall keys hold far more than installed software: update payloads,
/// servicing components, and the child entries of bundled installers. Windows
/// itself hides all of them from Add/Remove Programs. An inventory that does not
/// is mostly hotfix rows - on a patched server, thousands of them - and the actual
/// software is buried where nobody will find it.
pub fn is_reportable_product(entry: &UninstallEntry<'_>) -> bool {
    // No display name means the key is not something a person installed: orphaned
    // keys and update payloads look exactly like this.
    let Some(name) = entry.display_name else {
        return false;
    };
    if name.trim().is_empty() {
        return false;
    }
    if entry.system_component {
        return false;
    }
    // A child of a bundled installer is already represented by its parent, and
    // listing both double-counts every suite.
    if entry
        .parent_key_name
        .is_some_and(|value| !value.trim().is_empty())
    {
        return false;
    }
    if let Some(release) = entry.release_type {
        let release = release.to_lowercase();
        if release.contains("update")
            || release.contains("hotfix")
            || release.contains("securityupdate")
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_eleven_is_not_reported_as_windows_ten() {
        assert_eq!(
            edition_name(Some("Windows 10 Pro"), Some("22631")),
            "Windows 11 Pro"
        );
        assert_eq!(
            edition_name(Some("Windows 10 Enterprise"), Some("22000")),
            "Windows 11 Enterprise"
        );
        // Genuine Windows 10 stays Windows 10.
        assert_eq!(
            edition_name(Some("Windows 10 Enterprise LTSC"), Some("19045")),
            "Windows 10 Enterprise LTSC"
        );
        // Server builds share the numbering and must not be renamed.
        assert_eq!(
            edition_name(Some("Windows Server 2022 Standard"), Some("20348")),
            "Windows Server 2022 Standard"
        );
        assert_eq!(
            edition_name(Some("Windows Server 2025 Datacenter"), Some("26100")),
            "Windows Server 2025 Datacenter"
        );
        // An unreadable build must not invent a version.
        assert_eq!(edition_name(Some("Windows 10 Pro"), None), "Windows 10 Pro");
        assert_eq!(edition_name(None, Some("22631")), "Windows");
    }

    #[test]
    fn a_share_of_nothing_is_not_zero_percent() {
        assert_eq!(percent(0, 0), None);
        assert_eq!(percent(1, 4), Some(25));
        assert_eq!(percent(u64::MAX, u64::MAX), Some(100));
    }

    #[test]
    fn interface_types_are_named_not_numbered() {
        assert_eq!(interface_type_name(6), "ethernet");
        assert_eq!(interface_type_name(71), "wireless");
        assert_eq!(interface_type_name(9999), "other");
    }

    fn entry(name: Option<&str>) -> UninstallEntry<'_> {
        UninstallEntry {
            display_name: name,
            system_component: false,
            release_type: None,
            parent_key_name: None,
        }
    }

    #[test]
    fn reports_products_and_hides_what_windows_hides() {
        assert!(is_reportable_product(&entry(Some("7-Zip 24.09"))));

        // These four are why an unfiltered inventory is unusable.
        assert!(!is_reportable_product(&entry(None)));
        assert!(!is_reportable_product(&entry(Some("   "))));
        assert!(!is_reportable_product(&UninstallEntry {
            system_component: true,
            ..entry(Some("Microsoft Visual C++ 2015 Runtime"))
        }));
        assert!(!is_reportable_product(&UninstallEntry {
            release_type: Some("Security Update"),
            ..entry(Some("Update for Microsoft Office"))
        }));
        assert!(!is_reportable_product(&UninstallEntry {
            release_type: Some("Hotfix"),
            ..entry(Some("Hotfix for Windows"))
        }));
        assert!(!is_reportable_product(&UninstallEntry {
            parent_key_name: Some("Office16"),
            ..entry(Some("Office 16 Click-to-Run Extensibility Component"))
        }));

        // An empty ReleaseType or ParentKeyName is not a reason to hide a product.
        assert!(is_reportable_product(&UninstallEntry {
            release_type: Some(""),
            parent_key_name: Some(""),
            ..entry(Some("Notepad++"))
        }));
    }
}
