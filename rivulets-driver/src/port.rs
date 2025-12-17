use core::marker::PhantomData;
use core::ops::Deref;

use crate::databus::{Consumer, Producer, Transformer};
use crate::payload::{Metadata, ReadPayload, TransformPayload, WritePayload};

/// Represents an input port for an `Element`.
pub enum InPort<C: Consumer> {
    Consumer(C),
    None,
}

impl<T> InPort<Dmy<T>> {
    pub fn new_none() -> Self {
        InPort::None
    }
}

impl<T> Default for InPort<Dmy<T>> {
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
pub enum OutPort<P: Producer> {
    Producer(P),
    None,
}

impl<T> OutPort<Dmy<T>> {
    pub fn new_none() -> Self {
        OutPort::None
    }
}

impl<T> Default for OutPort<Dmy<T>> {
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
pub enum InPlacePort<T: Transformer> {
    Transformer(T),
    None,
}

impl<T> InPlacePort<Dmy<T>> {
    pub fn new_none() -> Self {
        InPlacePort::None
    }
}

impl<T> Default for InPlacePort<Dmy<T>> {
    fn default() -> Self {
        InPlacePort::None
    }
}

/// The data transfer requirements for an `Element`'s port.
/// Specifies the minimum required payload size for each port type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRequirements {
    pub in_: Option<PayloadSize>,
    /// Specifies the output requirement. This slot is also used for in-place transformations.
    pub out: Option<PayloadSize>,
    /// Indicates whether the element supports in-place processing.
    pub in_place: bool,
    /// Indicates whether the producer requires the buffer to be physically contiguous
    /// and exactly the requested size.
    pub strict_alloc: bool,
    /// Indicates whether the consumer is expected to consume the entire read payload.
    pub consume_all: bool,
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
    pub fn new(in_: Option<PayloadSize>, out: Option<PayloadSize>, in_place: bool, strict_alloc: bool, consume_all: bool) -> Self {
        Self {
            in_: in_,
            out: out,
            in_place,
            strict_alloc,
            consume_all,
        }
    }

    pub fn new_payload_to_payload(in_size: PayloadSize, out_size: PayloadSize, strict_alloc: bool, consume_all: bool) -> Self {
        Self {
            in_: Some(in_size),
            out: Some(out_size),
            in_place: false,
            strict_alloc,
            consume_all,
        }
    }

    pub fn new_in_place(size: PayloadSize, strict_alloc: bool, consume_all: bool) -> Self {
        Self {
            in_: None,
            out: Some(size),
            in_place: true,
            strict_alloc,
            consume_all,
        }
    }

    pub fn sink(size: PayloadSize, strict_alloc: bool) -> Self {
        Self {
            in_: Some(size),
            out: None,
            in_place: false,
            strict_alloc,
            consume_all: false,
        }
    }

    pub fn source(size: PayloadSize, strict_alloc: bool) -> Self {
        Self {
            in_: None,
            out: Some(size),
            in_place: false,
            strict_alloc,
            consume_all: false,
        }
    }
}

/// A dummy struct used as a placeholder for unused generic type parameters
/// in `InPort` and `OutPort`.
#[derive(Debug, Clone, Copy)]
pub struct Dmy<T = u8>(PhantomData<T>);

impl<T> Dmy<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for Dmy<T> {
    fn default() -> Self {
        Self::new()
    }
}

// --- Dummy Trait Implementations for Dmy ---

impl<'a, T> crate::databus::Databus for Dmy<T> {
    type Item = T;

    fn do_register_producer(&self, _payload_size: PayloadSize, _strict_alloc: bool) {
        unimplemented!()
    }
    fn do_register_consumer(&self, _payload_size: PayloadSize, _consume_all: bool) -> u8 {
        unimplemented!()
    }
    fn do_register_transformer(
        &self,
        _payload_size: PayloadSize,
        _strict_alloc: bool,
        _consume_all: bool,
    ) {
        unimplemented!()
    }
}

impl<T> Consumer for Dmy<T> {
    type Item = T;

    async fn acquire_read<'a>(&'a self, _len: usize) -> ReadPayload<'a, Self> {
        unimplemented!()
    }
    fn release_read(&self, _metadata: Metadata, _consumed_len: usize) {
        unimplemented!()
    }
}

impl<T> Producer for Dmy<T> {
    type Item = T;

    async fn acquire_write<'a>(&'a self, _len: usize) -> WritePayload<'a, Self> {
        unimplemented!()
    }
    fn release_write(&self, _metadata: Metadata, _written_len: usize) {
        unimplemented!()
    }
}

impl<T> Transformer for Dmy<T> {
    type Item = T;

    async fn acquire_transform<'a>(&'a self, _len: usize) -> TransformPayload<'a, Self> {
        unimplemented!()
    }
    fn release_transform(&self, _metadata: Metadata, _transformed_len: usize) {
        unimplemented!()
    }
}

impl<T> Deref for Dmy<T> {
    type Target = Self;
    fn deref(&self) -> &Self::Target {
        &self
    }
}
