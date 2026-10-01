//! Background load progress.
//!
//! Opening a large direct file or a ZIP entry is a two-stage wait:
//! decompression (ZIP only) and index building. Both stages can report
//! how many bytes they have processed, and the ZIP stage knows the
//! uncompressed size up front, so the UI can show a determinate bar
//! while indexing a known source shows bytes over the mmap length.
//!
//! The data side never touches the UI: it invokes a [`ProgressSink`]
//! callback with [`LoadProgress`] samples. Callers that do not care pass
//! `None`, and the reporting branch compiles to a cheap comparison.

/// Which stage of a load produced a [`LoadProgress`] sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPhase {
    /// Discovering the source: opening the file and reading a ZIP
    /// central directory.
    Reading,
    /// Decompressing a ZIP entry to its temp file.
    Extracting,
    /// Walking the bytes to build the record-offset index.
    Indexing,
}

impl LoadPhase {
    /// Stable tag for lock-free transport (the UI stores the phase in an
    /// `AtomicU8`).
    pub fn as_u8(self) -> u8 {
        match self {
            LoadPhase::Reading => 0,
            LoadPhase::Extracting => 1,
            LoadPhase::Indexing => 2,
        }
    }

    /// Inverse of [`as_u8`](Self::as_u8); unknown tags read as `Reading`.
    pub fn from_u8(tag: u8) -> Self {
        match tag {
            1 => LoadPhase::Extracting,
            2 => LoadPhase::Indexing,
            _ => LoadPhase::Reading,
        }
    }
}

/// One progress sample. `bytes_total` is `None` when the total is not
/// known ahead of time (the caller should show an indeterminate
/// indicator), and `Some` for a determinate bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadProgress {
    pub phase: LoadPhase,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
}

impl LoadProgress {
    /// Fraction complete in `[0.0, 1.0]`, or `None` when the total is
    /// unknown.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.bytes_total?;
        if total == 0 {
            return Some(1.0);
        }
        Some((self.bytes_done as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }
}

/// A callback invoked by the data side as a load advances. It must be
/// cheap: the indexer may call it a few times per mebibyte.
pub type ProgressSink<'a> = &'a (dyn Fn(LoadProgress) + Send + Sync);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_is_none_when_the_total_is_unknown() {
        let p = LoadProgress {
            phase: LoadPhase::Extracting,
            bytes_done: 10,
            bytes_total: None,
        };
        assert_eq!(p.fraction(), None);
    }

    #[test]
    fn fraction_clamps_and_handles_zero() {
        let half = LoadProgress {
            phase: LoadPhase::Indexing,
            bytes_done: 5,
            bytes_total: Some(10),
        };
        assert_eq!(half.fraction(), Some(0.5));

        let empty = LoadProgress {
            phase: LoadPhase::Indexing,
            bytes_done: 0,
            bytes_total: Some(0),
        };
        assert_eq!(empty.fraction(), Some(1.0));

        let over = LoadProgress {
            phase: LoadPhase::Indexing,
            bytes_done: 20,
            bytes_total: Some(10),
        };
        assert_eq!(over.fraction(), Some(1.0));
    }
}
