//! SMBIOS structure-table parsing.
//!
//! The firmware UUID is what identifies a physical or virtual machine across a
//! reinstall of its operating system, and it is normally read on Windows through
//! WMI's Win32_ComputerSystemProduct. Reading the raw table instead avoids a COM
//! apartment and a dependency on the WMI service, but it means owning the byte
//! layout - so the parsing lives here, apart from the Windows call that fetches
//! the table, and is tested on every platform.

/// Walks the structure table and returns the type 1 (System Information) UUID.
///
/// Returns None when there is no type 1 structure, when its UUID field is absent,
/// or when the field is one of the two "not set" patterns. That last case matters:
/// a great many machines ship with an all-zero or all-ones UUID, and reporting it
/// would make every one of them look like the same host to the Server.
pub fn system_uuid(table: &[u8]) -> Option<String> {
    let mut offset = 0usize;
    while offset + 4 <= table.len() {
        let kind = table[offset];
        let length = table[offset + 1] as usize;
        // A length below the 4-byte header, or one that runs past the table, means
        // the table is malformed and nothing after this point can be trusted.
        if length < 4 || offset + length > table.len() {
            return None;
        }
        if kind == 1 && length >= 24 {
            let uuid = &table[offset + 8..offset + 24];
            if uuid.iter().all(|&byte| byte == 0) || uuid.iter().all(|&byte| byte == 0xFF) {
                return None;
            }
            return Some(format_uuid(uuid));
        }
        // The formatted part is followed by a string area terminated by two NULs.
        let mut cursor = offset + length;
        while cursor + 1 < table.len() && !(table[cursor] == 0 && table[cursor + 1] == 0) {
            cursor += 1;
        }
        let next = cursor + 2;
        if next <= offset {
            return None;
        }
        offset = next;
    }
    None
}

/// SMBIOS 2.6 and later store the first three groups of the UUID little-endian,
/// which is why this is not a straight hex dump: getting it wrong produces a
/// plausible-looking UUID that matches nothing the hypervisor or the hardware
/// vendor reports.
fn format_uuid(bytes: &[u8]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[3],
        bytes[2],
        bytes[1],
        bytes[0],
        bytes[5],
        bytes[4],
        bytes[7],
        bytes[6],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds one structure: type, length, handle, body, then the string area.
    fn structure(kind: u8, body: &[u8], strings: &[&str]) -> Vec<u8> {
        let mut bytes = vec![kind, (4 + body.len()) as u8, 0x01, 0x00];
        bytes.extend_from_slice(body);
        for text in strings {
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
        }
        // An empty string area is still terminated by two NULs.
        if strings.is_empty() {
            bytes.push(0);
        }
        bytes.push(0);
        bytes
    }

    fn type_one(uuid: [u8; 16]) -> Vec<u8> {
        // Type 1 body: manufacturer, product, version, serial string indexes,
        // then the 16-byte UUID at offset 8 of the structure.
        let mut body = vec![1u8, 2, 3, 4];
        body.extend_from_slice(&uuid);
        body.push(0x06); // wake-up type
        structure(
            1,
            &body,
            &["Contoso", "Virtual Machine", "1.0", "0000-1111"],
        )
    }

    #[test]
    fn reads_the_uuid_with_the_first_three_groups_byte_swapped() {
        // The bytes as firmware stores them for
        // 4c4c4544-0051-4d10-8058-b8c04f574a32, a real Dell-style value.
        let stored = [
            0x44, 0x45, 0x4c, 0x4c, 0x51, 0x00, 0x10, 0x4d, 0x80, 0x58, 0xb8, 0xc0, 0x4f, 0x57,
            0x4a, 0x32,
        ];
        let table = type_one(stored);
        assert_eq!(
            system_uuid(&table).as_deref(),
            Some("4c4c4544-0051-4d10-8058-b8c04f574a32")
        );
    }

    #[test]
    fn skips_structures_before_type_one() {
        // A BIOS Information structure comes first on real hardware, and its
        // string area has to be walked correctly to find anything after it.
        let mut table = structure(0, &[1u8, 2, 0x00, 0xF0, 3, 0x40], &["Contoso", "2.13.0"]);
        table.extend_from_slice(&type_one([
            0x78, 0x56, 0x34, 0x12, 0x34, 0x12, 0x78, 0x56, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22,
            0x33, 0x44,
        ]));
        assert_eq!(
            system_uuid(&table).as_deref(),
            Some("12345678-1234-5678-9abc-def011223344")
        );
    }

    /// A very large number of machines ship with one of these, and reporting it
    /// would collapse the whole estate onto a single identity.
    #[test]
    fn treats_an_unset_uuid_as_absent() {
        assert!(system_uuid(&type_one([0x00; 16])).is_none());
        assert!(system_uuid(&type_one([0xFF; 16])).is_none());
    }

    #[test]
    fn refuses_a_malformed_table_instead_of_reading_past_it() {
        // A length shorter than the header, a length past the end, and a table
        // with no type 1 at all must all answer None rather than panic.
        assert!(system_uuid(&[1, 2, 0, 0]).is_none());
        assert!(system_uuid(&[1, 200, 0, 0, 0, 0]).is_none());
        assert!(system_uuid(&structure(0, &[1, 2], &["only bios"])).is_none());
        assert!(system_uuid(&[]).is_none());
        // A truncated type 1 whose declared length does not reach the UUID.
        assert!(system_uuid(&structure(1, &[1, 2, 3, 4], &[])).is_none());
    }
}
