//! Guards a rule that only bites on Windows, from a test that runs everywhere.
//!
//! `src/windows_sys.rs` compiles only under `#[cfg(windows)]`, so its unit tests
//! never run on the machines this project is developed and reviewed on. The rule
//! they would protect is easy to break by accident and expensive when broken: a
//! Win32 length reported by the operating system must be clamped to the buffer
//! before it is used to slice, because slicing past the end panics and a panic in
//! a collector costs that host its inventory.
//!
//! Reading the source is a poor substitute for running the code, and it cannot
//! see through a rename or a differently shaped expression. It does catch the
//! thing that actually happened - somebody writing the obvious `&buffer[..len]`
//! against a length Win32 handed back - which is worth having on every platform.

use std::path::Path;

/// Names that hold a length reported by Win32 rather than one we computed.
const OS_REPORTED: [&str; 4] = ["length", "written", "size", "count"];

#[test]
fn os_reported_lengths_are_clamped_before_slicing() {
    let source = std::fs::read_to_string(Path::new("src/windows_sys.rs"))
        .expect("src/windows_sys.rs must be readable from the crate root");

    let mut offenders = Vec::new();
    for (number, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("").trim();
        for name in OS_REPORTED {
            // `[..length as usize]`, `[..written as usize]`, and the `usize`-typed
            // forms without the cast.
            for pattern in [
                format!("[..{name} as usize]"),
                format!("[..{name}]"),
                format!("[..{name} as usize)"),
            ] {
                if code.contains(&pattern) {
                    offenders.push(format!("  src/windows_sys.rs:{}: {code}", number + 1));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these slice with a length Win32 reported instead of clamping it with take():\n{}\n\n\
         Win32 does not promise the length fits: GetLogicalDriveStringsW returns the\n\
         required size when the buffer was too small, and a driver may report a\n\
         hardware address longer than the fixed field holding it. Use take(buffer, len).",
        offenders.join("\n")
    );
}
