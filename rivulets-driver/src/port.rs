use core::ops::Deref;

use crate::databus::{Consumer, Producer, Transformer};
use crate::payload::{Metadata, ReadPayload, TransformPayload, WritePayload};

/// Represents an input port for an `Element`.
///
/// An `Element` can receive data from a `Consumer`, which provides data payloads.
pub enum InPort<C: Consumer> {
    /// An upstream databus component that implements the `Consumer` trait.
    Consumer(C),
    /// Represents no input, typically for source elements like generators.
    None,
}

impl InPort<Dmy> {
    pub fn new_none() -> Self {
        InPort::None
    }
}

impl Default for InPort<Dmy> {
    fn default() -> Self {
        InPort::None
    }
}

impl<C: Consumer> InPort<C> {
    pub fn unwrap(self) -> C {
        match self {
            Self::Consumer(val) => val,
            _ => panic!("called `InPort::unwrap()` on a `None` value"),
        }
    }

    pub fn consumer_ref(&self) -> &C {
        match self {
            Self::Consumer(val) => val,
            _ => panic!("called `InPort::as_ref()` on a `None` value"),
        }
    }
}

/// Represents an output port for an `Element`.
///
/// An `Element` can send data to a `Producer`, which accepts data payloads.
pub enum OutPort<P: Producer> {
    /// A downstream databus component that implements the `Producer` trait.
    Producer(P),
    /// Represents no output, typically for sink elements.
    None,
}

impl OutPort<Dmy> {
    pub fn new_none() -> Self {
        OutPort::None
    }
}

impl Default for OutPort<Dmy> {
    fn default() -> Self {
        OutPort::None
    }
}

impl<P: Producer> OutPort<P> {
    pub fn unwrap(self) -> P {
        match self {
            Self::Producer(val) => val,
            _ => panic!("called `OutPort::unwrap()` on a `None` value"),
        }
    }

    pub fn producer_ref(&self) -> &P {
        match self {
            Self::Producer(val) => val,
            _ => panic!("called `InPort::as_ref()` on a `None` value"),
        }
    }
}

/// Represents an in-place transformation port for an `Element`.
///
/// An `Element` can perform in-place transformations using a `Transformer`.
/// This is typically used for effects or filters that modify data in place.
pub enum InPlacePort<T: Transformer> {
    /// A databus component that implements the `Transformer` trait.
    Transformer(T),
    /// Represents no in-place transformation.
    None,
}

impl InPlacePort<Dmy> {
    pub fn new_none() -> Self {
        InPlacePort::None
    }
}

impl Default for InPlacePort<Dmy> {
    fn default() -> Self {
        InPlacePort::None
    }
}

/// The data transfer requirements for an `Element`'s port.
/// Specifies the minimum required payload size for each port type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRequirements {
    pub in_: Option<PayloadSize>,
    pub out: Option<PayloadSize>,
    pub in_place: Option<PayloadSize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadSize {
    pub min: u16,
    pub preferred: u16,
}

impl PayloadSize {
    pub fn new(min: u16, preferred: u16) -> Self {
        Self { min, preferred }
    }
}

impl PortRequirements {
    pub fn new() -> Self {
        Self {
            in_: None,
            out: None,
            in_place: None,
        }
    }

    pub fn new_payload_to_payload(in_size: PayloadSize, out_size: PayloadSize) -> Self {
        Self {
            in_: Some(in_size),
            out: Some(out_size),
            in_place: None,
        }
    }

    pub fn new_in_place(size: PayloadSize) -> Self {
        Self {
            in_: None,
            out: None,
            in_place: Some(size),
        }
    }

    pub fn sink(size: PayloadSize) -> Self {
        Self {
            in_: Some(size),
            out: None,
            in_place: None,
        }
    }

    pub fn source(size: PayloadSize) -> Self {
        Self {
            in_: None,
            out: Some(size),
            in_place: None,
        }
    }
}

/// A dummy struct used as a placeholder for unused generic type parameters
/// in `InPort` and `OutPort`.
#[derive(Debug, Clone, Copy)]
pub struct Dmy;

// --- Dummy Trait Implementations for Dmy ---

impl<'a> crate::databus::Databus for Dmy {
    type Item = u8;

    fn do_register_producer(&self, _payload_size: PayloadSize) {
        unimplemented!()
    }
    fn do_register_consumer(&self, _payload_size: PayloadSize) -> u8 {
        unimplemented!()
    }
    fn do_register_transformer(&self, _payload_size: PayloadSize) {
        unimplemented!()
    }
}

impl Consumer for Dmy {
    type Item = u8;

    async fn acquire_read<'a>(&'a self, _len: usize) -> ReadPayload<'a, Self> {
        unimplemented!()
    }
    fn release_read(&self, _consumed_bytes: usize) {
        unimplemented!()
    }
}

impl Producer for Dmy {
    type Item = u8;

    async fn acquire_write<'a>(&'a self, _len: usize, _exact: bool) -> WritePayload<'a, Self> {
        unimplemented!()
    }
    fn release_write(&self, _metadata: Metadata) {
        unimplemented!()
    }
}

impl Transformer for Dmy {
    type Item = u8;

    async fn acquire_transform<'a>(&'a self, _len: usize) -> TransformPayload<'a, Self> {
        unimplemented!()
    }
    fn release_transform(&self, _metadata: Metadata, _remaining_length: usize) {
        unimplemented!()
    }
}

impl Deref for Dmy {
    type Target = Self;
    fn deref(&self) -> &Self::Target {
        &self
    }
}
