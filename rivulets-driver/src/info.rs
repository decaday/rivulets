//! Defines the trait for stream metadata.

/// A trait for structures that describe the properties of a data stream.
///
/// This trait can be implemented by any struct that holds metadata,
/// allowing a pipeline to be configured based on the nature of the data
/// flowing through it.
pub trait Info: core::fmt::Debug + Clone + Copy + PartialEq + Eq {
    // fn is_compatible_with(&self, other: &Self) -> bool;
    fn vaild(&self) -> bool {
        true
    }

    fn get_alignment_bytes(&self) -> u8;
}

/// A default, empty Info struct for pipelines that do not need metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyInfo;

impl Info for EmptyInfo {
    fn vaild(&self) -> bool {
        true
    }

    fn get_alignment_bytes(&self) -> u8 {
        1
    }
}