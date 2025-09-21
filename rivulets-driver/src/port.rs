use crate::databus::{Consumer, Producer, Transformer};
use crate::payload::{Metadata, ReadPayload, TransformPayload, WritePayload};

/// Represents an input port for an `Element`.
///
/// An `Element` can receive data from a `Consumer`, which provides data payloads.
pub enum InPort<'a, C: Consumer<'a>> {
    /// An upstream databus component that implements the `Consumer` trait.
    Consumer(&'a C),
    /// Represents no input, typically for source elements like generators.
    None,
}

impl InPort<'_, Dmy> {
    pub fn new_none() -> Self {
        InPort::None
    }
}

impl Default for InPort<'_, Dmy> {
    fn default() -> Self {
        InPort::None
    }
}

/// Represents an output port for an `Element`.
///
/// An `Element` can send data to a `Producer`, which accepts data payloads.
pub enum OutPort<'a, P: Producer<'a>> {
    /// A downstream databus component that implements the `Producer` trait.
    Producer(&'a P),
    /// Represents no output, typically for sink elements.
    None,
}

impl OutPort<'_, Dmy> {
    pub fn new_none() -> Self {
        OutPort::None
    }
}

impl Default for OutPort<'_, Dmy> {
    fn default() -> Self {
        OutPort::None
    }
}

/// Represents an in-place transformation port for an `Element`.
///
/// An `Element` can perform in-place transformations using a `Transformer`.
/// This is typically used for effects or filters that modify data in place.
pub enum InPlacePort<'a, T: Transformer<'a>> {
    /// A databus component that implements the `Transformer` trait.
    Transformer(&'a T),
    /// Represents no in-place transformation.
    None,
}

impl InPlacePort<'_, Dmy> {
    pub fn new_none() -> Self {
        InPlacePort::None
    }
}

impl Default for InPlacePort<'_, Dmy> {
    fn default() -> Self {
        InPlacePort::None
    }
}

/// The data transfer requirements for an `Element`'s port.
/// Specifies the minimum required payload size for each port type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRequirements {
    pub in_payload: Option<u16>,
    pub out_payload: Option<u16>,
    pub in_place: Option<u16>,
}

impl PortRequirements {
    pub fn new() -> Self {
        Self {
            in_payload: None,
            out_payload: None,
            in_place: None,
        }
    }

    pub fn new_payload_to_payload(in_size: u16, out_size: u16) -> Self {
        Self {
            in_payload: Some(in_size),
            out_payload: Some(out_size),
            in_place: None,
        }
    }

    pub fn new_in_place(size: u16) -> Self {
        Self {
            in_payload: None,
            out_payload: None,
            in_place: Some(size),
        }
    }

    pub fn sink(size: u16) -> Self {
        Self {
            in_payload: Some(size),
            out_payload: None,
            in_place: None,
        }
    }

    pub fn source(size: u16) -> Self {
        Self {
            in_payload: None,
            out_payload: Some(size),
            in_place: None,
        }
    }
}

/// A dummy struct used as a placeholder for unused generic type parameters
/// in `InPort` and `OutPort`.
pub struct Dmy;

// --- Dummy Trait Implementations for Dmy ---

impl<'a> Consumer<'a> for Dmy {
    async fn acquire_read(&'a self) -> ReadPayload<'a, Self> {
        unimplemented!()
    }
    fn release_read(&self, _consumed_bytes: usize) {
        unimplemented!()
    }
}

impl<'a> Producer<'a> for Dmy {
    async fn acquire_write(&'a self) -> WritePayload<'a, Self> {
        unimplemented!()
    }
    fn release_write(&self, _buf: &'a mut [u8], _metadata: Metadata) {
        unimplemented!()
    }
}

impl<'a> Transformer<'a> for Dmy {
    async fn acquire_transform(&'a self) -> TransformPayload<'a, Self> {
        unimplemented!()
    }
    fn release_transform(&self, _buf: &'a mut [u8], _metadata: Metadata) {
        unimplemented!()
    }
}
