use rivulets_driver::format::Format;
use rivulets_driver::port::PayloadSize;
use rivulets_driver::databus::{Consumer, Databus, DatabusRef, Producer};
use rivulets_driver::node::Node;

use crate::databus::{ConsumerHandle, ProducerHandle};

pub struct Config {
    pub prefer_items_per_process: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self { 
            prefer_items_per_process: 64, 
        }
    }
}

pub struct ChannelSplitter2<DI, DO1, DO2, F, T>
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

impl<DI, DO1, DO2, F, T> ChannelSplitter2<DI, DO1, DO2, F, T>
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

        Self {
            consumer: ConsumerHandle::new(databus_in, PayloadSize::new(min, preferred)),
            producer1: ProducerHandle::new(databus_out1, PayloadSize::new(min, preferred)),
            producer2: ProducerHandle::new(databus_out2, PayloadSize::new(min, preferred)),
            _format: in_format,
            config,
        }
    }
}

impl<DI, DO1, DO2, F, T> Node for ChannelSplitter2<DI, DO1, DO2, F, T>
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
            
            let mut read_payload = self.consumer.acquire_read(payload_size).await;
            let actual_len = read_payload.len();

            let mut write_payload1 = self.producer1.acquire_write(actual_len, true).await;
            let mut write_payload2 = self.producer2.acquire_write(actual_len, true).await;

            write_payload1[..actual_len].copy_from_slice(&read_payload[..actual_len]);
            write_payload2[..actual_len].copy_from_slice(&read_payload[..actual_len]);

            read_payload.commit(actual_len);
            write_payload1.commit(actual_len);
            write_payload2.commit(actual_len);
            
            write_payload1.set_position(read_payload.position());
            write_payload2.set_position(read_payload.position());
        }
    }
}
