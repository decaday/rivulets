use rivulets_driver::format::Format;
use rivulets_driver::port::PayloadSize;
use rivulets_driver::databus::{Consumer, Databus, DatabusRef, Producer};
use rivulets_driver::node::Node;

use crate::databus::{ConsumerHandle, ProducerHandle};

pub use crate::StandardConfig as Config;

pub struct Splitter2<DI, DO1, DO2, F, T>
where
    DI: DatabusRef,
    DO1: DatabusRef,
    DO2: DatabusRef,
    DI::Databus: Databus<Item = T>,
    DO1::Databus: Databus<Item = T>,
    DO2::Databus: Databus<Item = T>,
    ConsumerHandle<DI::Databus, DI>: Consumer<Item = T>,
    ProducerHandle<DO1::Databus, DO1>: Producer<Item = T>,
    ProducerHandle<DO2::Databus, DO2>: Producer<Item = T>,
    F: Format,
    T: Copy + 'static,
{
    consumer: ConsumerHandle<DI::Databus, DI>,
    producer1: ProducerHandle<DO1::Databus, DO1>,
    producer2: ProducerHandle<DO2::Databus, DO2>,
    _format: F,
    config: Config,
}

impl<DI, DO1, DO2, F, T> Splitter2<DI, DO1, DO2, F, T>
where
    DI: DatabusRef,
    DO1: DatabusRef,
    DO2: DatabusRef,
    DI::Databus: Databus<Item = T>,
    DO1::Databus: Databus<Item = T>,
    DO2::Databus: Databus<Item = T>,
    ConsumerHandle<DI::Databus, DI>: Consumer<Item = T>,
    ProducerHandle<DO1::Databus, DO1>: Producer<Item = T>,
    ProducerHandle<DO2::Databus, DO2>: Producer<Item = T>,
    F: Format,
    T: Copy + 'static,
{
    pub fn new(databus_in: DI, databus_out1: DO1, databus_out2: DO2, in_format: F, config: Config) -> Self {
        assert!(in_format.is_mono());
        
        let min = 1;
        let preferred = config.prefer_items_per_process;
        let payload_size = PayloadSize::new(min, preferred);
        let strict_alloc = false;
        let consume_all = false;

        Self {
            consumer: ConsumerHandle::new(databus_in, payload_size, consume_all),
            producer1: ProducerHandle::new(databus_out1, payload_size, strict_alloc),
            producer2: ProducerHandle::new(databus_out2, payload_size, strict_alloc),
            _format: in_format,
            config,
        }
    }
}

impl<DI, DO1, DO2, F, T> Node for Splitter2<DI, DO1, DO2, F, T>
where
    DI: DatabusRef,
    DO1: DatabusRef,
    DO2: DatabusRef,
    DI::Databus: Databus<Item = T>,
    DO1::Databus: Databus<Item = T>,
    DO2::Databus: Databus<Item = T>,
    ConsumerHandle<DI::Databus, DI>: Consumer<Item = T>,
    ProducerHandle<DO1::Databus, DO1>: Producer<Item = T>,
    ProducerHandle<DO2::Databus, DO2>: Producer<Item = T>,
    F: Format,
    T: Copy + 'static,
{
    type Error = ();

    async fn init(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn run(&mut self) -> Result<(), Self::Error> {
        loop {
            let payload_size = self.config.prefer_items_per_process as usize;
            
            let mut write_payload1 = self.producer1.acquire_write(payload_size).await;
            let mut write_payload2 = self.producer2.acquire_write(payload_size).await;

            let write_len = write_payload1.len().min(write_payload2.len());
            let mut read_payload = self.consumer.acquire_read(write_len).await;
            let process_len = read_payload.len();

            write_payload1[..process_len].copy_from_slice(&read_payload[..process_len]);
            write_payload2[..process_len].copy_from_slice(&read_payload[..process_len]);

            read_payload.commit(process_len);
            write_payload1.commit(process_len);
            write_payload2.commit(process_len);
            
            write_payload1.set_position(read_payload.position());
            write_payload2.set_position(read_payload.position());
        }
    }
}
