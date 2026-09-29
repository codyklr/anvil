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
