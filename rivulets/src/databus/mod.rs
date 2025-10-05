#[cfg(feature="alloc")]
use std::sync::Arc;

use rivulets_driver::databus::Databus;
use rivulets_driver::port::PayloadSize;

pub mod slot;

pub struct ProducerHandle<D: Databus> {
    #[cfg(not(feature="alloc"))]
    pub inner: &'static D,
    #[cfg(feature="alloc")]
    pub inner: Arc<D>,
}

pub struct ConsumerHandle<D: Databus> {
    #[cfg(not(feature="alloc"))]
    pub inner: &'static D,
    #[cfg(feature="alloc")]
    pub inner: Arc<D>,
    pub id: u8,
}

pub struct TransformerHandle<D: Databus> {
    #[cfg(not(feature="alloc"))]
    pub inner: &'static D,
    #[cfg(feature="alloc")]
    pub inner: Arc<D>,
}

impl<D: Databus> ProducerHandle<D> {
    #[cfg(not(feature="alloc"))]
    pub fn new(databus: &'static D, payload_size: PayloadSize) -> Self {
        databus.do_register_producer(payload_size);
        Self { inner: databus }
    }

    #[cfg(feature="alloc")]
    pub fn new(databus: Arc<D>, payload_size: PayloadSize) -> Self {
        databus.do_register_producer(payload_size);
        Self { inner: databus }
    }
}

impl<D: Databus> ConsumerHandle<D> {
    #[cfg(not(feature="alloc"))]
    pub fn new(databus: &'static D, payload_size: PayloadSize) -> Self {
        let id = databus.do_register_consumer(payload_size);
        Self { inner: databus, id }
    }

    #[cfg(feature="alloc")]
    pub fn new(databus: Arc<D>, payload_size: PayloadSize) -> Self {
        let id = databus.do_register_consumer(payload_size);
        Self { inner: databus, id }
    }
}

impl<D: Databus> TransformerHandle<D> {
    #[cfg(not(feature="alloc"))]
    pub fn new(databus: &'static D, payload_size: PayloadSize) -> Self {
        databus.do_register_transformer(payload_size);
        Self { inner: databus }
    }

    #[cfg(feature="alloc")]
    pub fn new(databus: Arc<D>, payload_size: PayloadSize) -> Self {
        databus.do_register_transformer(payload_size);
        Self { inner: databus }
    }
}
