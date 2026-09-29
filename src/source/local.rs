//! Local filesystem source. Most of the logic for direct file opening lives
//! in `mod.rs`; this module is a placeholder for things like "follow symlink"
//! that we'll likely add later.

use std::path::Path;

use crate::error::Result;
use crate::source::{MmapSource, OpenedSource, SourceKind};

pub fn open_local(path: &Path) -> Result<OpenedSource> {
    let physical = MmapSource::open(path)?;
    Ok(OpenedSource {
        source: crate::source::DataSource {
            kind: SourceKind::LocalFile,
            path: path.to_path_buf(),
        },
        physical: std::sync::Arc::new(physical),
        archive_candidates: None,
        temp_dir: None,
    })
}
