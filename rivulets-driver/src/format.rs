//! Defines the trait for stream format metadata.

/// A trait for structures that describe the format properties of a data stream.
///
/// `Format` describes the structure of the stream, such as channel count,
/// independent of the underlying data type.
pub trait Format: core::fmt::Debug + Clone + Copy + PartialEq + Eq {
    fn valid(&self) -> bool {
        true
    }

    /// Returns the number of channels in the stream.
    /// Defaults to 1 (Mono).
    fn channel_count(&self) -> u8 {
        1
    }

    fn is_mono(&self) -> bool {
        self.channel_count() == 1
    }
}

/// A default, empty Format struct for pipelines that do not need metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyFormat;

impl Format for EmptyFormat {
    fn valid(&self) -> bool {
        true
    }
    
    fn channel_count(&self) -> u8 {
        1
    }
}