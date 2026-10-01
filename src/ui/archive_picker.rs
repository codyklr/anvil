//! Presentation helpers for archive contents.
//!
//! The actual overlay element is rendered by `view.rs` so it can be wired
//! directly to `MainView`'s selection state. This module owns the data
//! shaping: which entries are usable, how sizes are formatted, and how
//! entries are ordered.

use anvil_core::ArchiveCandidate;

/// Formats a byte count with binary units. Kept terse for the status bar.
pub fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Splits candidates into (usable, rejected). Usable entries keep the
/// original archive order; rejected entries are sorted by name so the
/// diagnostics list is stable.
pub fn partition(
    candidates: &[ArchiveCandidate],
) -> (Vec<&ArchiveCandidate>, Vec<&ArchiveCandidate>) {
    let mut usable = Vec::new();
    let mut rejected = Vec::new();
    for c in candidates {
        if c.usable {
            usable.push(c);
        } else {
            rejected.push(c);
        }
    }
    rejected.sort_by(|a, b| a.entry_name.cmp(&b.entry_name));
    (usable, rejected)
}

/// A rejected archive entry prepared for display: the entry name plus the
/// human-readable reason it was skipped. Kept separate from
/// `ArchiveCandidate` so the diagnostics list is easy to unit-test without
/// a `ZipFile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    pub name: String,
    pub reason: String,
}

/// Rejected entries in stable (name-sorted) order, ready for the picker's
/// diagnostics section.
pub fn skipped_entries(candidates: &[ArchiveCandidate]) -> Vec<SkippedEntry> {
    partition(candidates)
        .1
        .into_iter()
        .map(|c| SkippedEntry {
            name: c.entry_name.clone(),
            reason: c.rejected_reason.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, usable: bool, reason: &str) -> ArchiveCandidate {
        ArchiveCandidate {
            entry_name: name.to_string(),
            compressed_size: 10,
            uncompressed_size: 100,
            usable,
            rank: if usable { 0 } else { u8::MAX },
            rejected_reason: reason.to_string(),
        }
    }

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.00 GiB");
    }

    #[test]
    fn skipped_entries_are_name_sorted_and_drop_usable_ones() {
        let candidates = [
            candidate("z.csv", true, ""),
            candidate("__MACOSX/._z.csv", false, "macOS metadata (__MACOSX)"),
            candidate(".DS_Store", false, "macOS metadata (.DS_Store)"),
        ];
        let skipped = skipped_entries(&candidates);
        assert_eq!(
            skipped,
            vec![
                SkippedEntry {
                    name: ".DS_Store".into(),
                    reason: "macOS metadata (.DS_Store)".into(),
                },
                SkippedEntry {
                    name: "__MACOSX/._z.csv".into(),
                    reason: "macOS metadata (__MACOSX)".into(),
                },
            ]
        );
    }

    #[test]
    fn skipped_entries_empty_when_everything_is_usable() {
        let candidates = [candidate("a.csv", true, ""), candidate("b.csv", true, "")];
        assert!(skipped_entries(&candidates).is_empty());
    }
}
