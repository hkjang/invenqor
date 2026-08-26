//! Guidance shown to an operator must name a flag this binary accepts.
//!
//! The Server had the matching defect: a diagnostic told operators to set
//! INVENQOR_MASTER_KEY, a variable that does not exist, so anyone who followed
//! it saw no change and had no reason to doubt the instruction. The Agent's
//! diagnose output is the same kind of surface - it prints commands for the
//! operator to run - and a flag renamed without updating the guidance would
//! fail the same way, silently and only for the person already in trouble.
//!
//! Every flag the Agent accepts is listed in src/main.rs, so both sides of the
//! comparison come from the source rather than from a list kept by hand.

use std::collections::BTreeSet;

fn flags_in(source: &str, pattern: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = source;
    while let Some(at) = rest.find(pattern) {
        rest = &rest[at + pattern.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_lowercase() || c == '-'))
            .unwrap_or(rest.len());
        let flag = &rest[..end];
        if flag.len() > 1 && !flag.ends_with('-') {
            found.insert(format!("--{flag}"));
        }
    }
    found
}

#[test]
fn every_flag_named_in_guidance_is_one_the_agent_accepts() {
    let main = std::fs::read_to_string("src/main.rs").expect("read src/main.rs");
    // Flags the parser matches, whether they stand alone or take a value.
    let accepted = flags_in(&main, "\"--");
    assert!(
        accepted.contains("--diagnose") && accepted.contains("--config"),
        "the flag list was not recognised, so this test proves nothing: {accepted:?}"
    );

    let mut offenders = Vec::new();
    for file in ["src/diagnose.rs", "src/health.rs", "src/scheduler.rs"] {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        for flag in flags_in(&source, "--") {
            if !accepted.contains(&flag) {
                offenders.push(format!("  {file}: {flag}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these are named in operator guidance but the Agent does not accept them:\n{}\n\n\
         An operator running the suggested command gets an error instead of the answer.",
        offenders.join("\n")
    );
}

/// Platform-specific commands belong in the one function that branches on the
/// platform.
///
/// A Windows service that could not read its config used to be told to run
/// sudo, chown and chmod - none of which exist there - because the start-up
/// path had its own copy of the Linux remedy while --diagnose had a properly
/// branched one. Both now call config_permission_remedy.
#[test]
fn platform_specific_commands_stay_inside_the_function_that_branches() {
    // diagnose.rs holds the branch itself, so it is the one place allowed to
    // name these. Anywhere else means a second copy that only fits one platform.
    let commands = ["sudo ", "chown ", "chmod 0", "icacls ", "Restart-Service"];
    let mut offenders = Vec::new();
    for file in [
        "src/main.rs",
        "src/scheduler.rs",
        "src/health.rs",
        "src/config.rs",
    ] {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        for (number, line) in source.lines().enumerate() {
            // Tests build their own fixtures and may name whatever they like.
            if line.trim_start().starts_with("//") {
                continue;
            }
            for command in commands {
                if line.contains(command) {
                    offenders.push(format!("  {file}:{}: {}", number + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these name a platform-specific command outside the function that \
         branches on the platform:\n{}\n\n\
         Call crate::diagnose::config_permission_remedy instead, so the reader \
         gets a command their machine actually has.",
        offenders.join("\n")
    );
}
