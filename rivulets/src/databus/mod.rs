use core::ops::Deref;

use rivulets_driver::databus::Databus;
use rivulets_driver::port::PayloadSize;

pub mod slot;

pub struct ProducerHandle<D: Databus, P: Deref<Target=D> + Clone> {
    pub inner: P,
}

pub struct ConsumerHandle<D: Databus, P: Deref<Target=D> + Clone> {
    pub inner: P,
    pub id: u8,
}

pub struct TransformerHandle<D: Databus, P: Deref<Target=D> + Clone> {
    pub inner: P,
}

impl<D: Databus, P: Deref<Target=D> + Clone> ProducerHandle<D, P> {
    pub fn new(databus: P, payload_size: PayloadSize) -> Self {
        databus.do_register_producer(payload_size);
        Self { inner: databus }
    }
}

impl<D: Databus, P: Deref<Target=D> + Clone> ConsumerHandle<D, P> {
    pub fn new(databus: P, payload_size: PayloadSize) -> Self {
        let id = databus.do_register_consumer(payload_size);
        Self { inner: databus, id }
    }
}

impl<D: Databus, P: Deref<Target=D> + Clone> TransformerHandle<D, P> {
    pub fn new(databus: P, payload_size: PayloadSize) -> Self {
        databus.do_register_transformer(payload_size);
        Self { inner: databus }
    }
}
