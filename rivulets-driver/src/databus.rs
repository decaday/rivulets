use core::ops::Deref;

use crate::payload::{Metadata, ReadPayload, TransformPayload, WritePayload};
use crate::port::{InPlacePort, InPort, OutPort, PayloadSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Operation {
    Produce = 0,
    Consume = 1,
    InPlace = 2,
}

impl Operation {
    pub const COUNT: usize = 3; 
}

pub trait Databus {
    type Item: Sized;
    fn do_register_producer(&self, payload_size: PayloadSize);
    fn do_register_consumer(&self, payload_size: PayloadSize) -> u8;
    fn do_register_transformer(&self, payload_size: PayloadSize);
}

/// A trait for components from which data can be read asynchronously.
#[allow(async_fn_in_trait)]
pub trait Consumer: Sized {
    type Item: Sized;

    /// Asynchronously acquires a payload for reading data.
    async fn acquire_read<'a>(&'a self, len: usize) -> ReadPayload<'a, Self>;

    /// Called by `ReadPayload` on drop.
    fn release_read(&self, metadata: Metadata, consumed_len: usize);

    fn in_port(self) -> InPort<Self> {
        InPort::Consumer(self)
    }
}

/// A trait for components to which data can be written asynchronously.
#[allow(async_fn_in_trait)]
pub trait Producer: Sized {
    type Item: Sized;

    /// Asynchronously acquires a payload for writing data.
    async fn acquire_write<'a>(&'a self, len: usize, exact: bool) -> WritePayload<'a, Self>;

    /// Called by `WritePayload` on drop.
    fn release_write(&self, metadata: Metadata, written_len: usize);

    fn out_port(self) -> OutPort<Self> {
        OutPort::Producer(self)
    }
}

/// A trait for components that support in-place modification.
#[allow(async_fn_in_trait)]
pub trait Transformer: Sized {
    type Item: Sized;

    /// Asynchronously acquires a payload for in-place transformation.
    async fn acquire_transform<'a>(&'a self, len: usize) -> TransformPayload<'a, Self>;

    /// Called by `TransformPayload` on drop.
    fn release_transform(&self, metadata: Metadata, transformed_len: usize);

    fn in_place_port(self) -> InPlacePort<Self> {
        InPlacePort::Transformer(self)
    }
}

pub trait DatabusRef: Deref<Target = Self::Databus> + Clone {
    type Databus: Databus + Sized;
}

impl<T> DatabusRef for T
where
    T: Deref + Clone,
    <T as Deref>::Target: Databus + Sized,
{
    type Databus = <T as Deref>::Target;
}