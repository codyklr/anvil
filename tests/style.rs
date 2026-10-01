//! Repository style guards.
//!
//! These are cheap text checks over the source tree. They exist so a
//! convention that is easy to violate by accident fails CI instead of
//! being caught by review.

use std::fs;
use std::path::{Path, PathBuf};

/// Collect every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Anvil source must not contain em-dashes (U+2014). They are a
/// typographic character that renders inconsistently in the UI and in
/// terminals; use a plain hyphen `-`, a colon, or a comma instead.
#[test]
fn source_contains_no_em_dashes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("tests"), &mut files);

    const EM_DASH: char = '\u{2014}';
    let mut offenders = Vec::new();
    for file in files {
        let text =
            fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for (line_no, line) in text.lines().enumerate() {
            if line.contains(EM_DASH) {
                offenders.push(format!("{}:{}", file.display(), line_no + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "em-dash (U+2014) found in source; use '-' or ':' instead:\n{}",
        offenders.join("\n")
    );
}
