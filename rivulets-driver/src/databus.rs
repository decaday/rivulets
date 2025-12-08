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
    fn do_register_producer(&self, payload_size: PayloadSize);
    fn do_register_consumer(&self, payload_size: PayloadSize) -> u8;
    fn do_register_transformer(&self, payload_size: PayloadSize);
}

/// A trait for components from which audio data can be read asynchronously.
#[allow(async_fn_in_trait)]
pub trait Consumer: Sized {
    /// Asynchronously acquires a payload for reading data.
    ///
    /// This function will wait until data is available in the databus.
    async fn acquire_read<'a>(&'a self, len: usize) -> ReadPayload<'a, Self>;

    /// Called by `ReadPayload` when it is dropped to release the buffer.
    ///
    /// This method is intended for internal use by the payload's RAII mechanism
    /// and should not be called directly by the user.
    fn release_read(&self, remaining_length: usize);

    // fn is_transformer_configured(&self) -> bool {
    //     false
    // }

    /// Helper to create an `InPort` for this `Consumer`.
    fn in_port(self) -> InPort<Self> {
        InPort::Consumer(self)
    }
}

/// A trait for components to which audio data can be written asynchronously.
#[allow(async_fn_in_trait)]
pub trait Producer: Sized {
    /// Asynchronously acquires a payload for writing data.
    ///
    /// This function will wait until space is available in the databus.
    async fn acquire_write<'a>(&'a self, len: usize, exact: bool) -> WritePayload<'a, Self>;

    /// Called by `WritePayload` when it is dropped to commit the written data.
    ///
    /// This method is intended for internal use by the payload's RAII mechanism
    /// and should not be called directly by the user.
    fn release_write(&self, metadata: Metadata);

    /// Helper to create an `OutPort` for this `Producer`.
    fn out_port(self) -> OutPort<Self> {
        OutPort::Producer(self)
    }
}

/// A trait for components that support in-place modification of a buffer.
#[allow(async_fn_in_trait)]
pub trait Transformer: Sized {
    /// Asynchronously acquires a payload for in-place transformation.
    ///
    /// This operation will wait until the databus contains data that is ready
    /// to be processed.
    async fn acquire_transform<'a>(&'a self, len: usize) -> TransformPayload<'a, Self>;

    /// Called by `TransformPayload` when it is dropped to finalize the transformation.
    ///
    /// This method is intended for internal use by the payload's RAII mechanism
    /// and should not be called directly by the user.
    fn release_transform(&self, metadata: Metadata, remaining_length: usize);

    /// Helper to create an `InPlacePort` for this `Transformer`.
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