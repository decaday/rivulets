use rivulets_driver::info::Info;
use rivulets_driver::port::PayloadSize;
use rivulets_driver::databus::{Consumer, Databus, DatabusRef, Producer};
use rivulets_driver::node::Node;

use crate::databus::{ConsumerHandle, ProducerHandle};

pub struct Config {
    pub prefer_samples_per_process: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self { 
            prefer_samples_per_process: 64, 
        }
    }
}

/// A node that splits (duplicates) a single input stream into two output streams.
///
/// Requires that the data type `T` implements `Copy`.
pub struct ChannelSplitter2<DI, DO1, DO2, INFO, T>
where
    DI: DatabusRef,
    DO1: DatabusRef,
    DO2: DatabusRef,
    // Ensure all databuses carry the same Item type T
    DI::Databus: Databus<Item = T>,
    DO1::Databus: Databus<Item = T>,
    DO2::Databus: Databus<Item = T>,
    
    ConsumerHandle<DI::Databus, DI>: Consumer<Item = T>,
    ProducerHandle<DO1::Databus, DO1>: Producer<Item = T>,
    ProducerHandle<DO2::Databus, DO2>: Producer<Item = T>,
    INFO: Info,
    T: Copy + 'static,
{
    consumer: ConsumerHandle<DI::Databus, DI>,
    producer1: ProducerHandle<DO1::Databus, DO1>,
    producer2: ProducerHandle<DO2::Databus, DO2>,
    info: INFO,
    config: Config,
}

impl<DI, DO1, DO2, INFO, T> ChannelSplitter2<DI, DO1, DO2, INFO, T>
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
    INFO: Info,
    T: Copy + 'static,
{
    pub fn new(databus_in: DI, databus_out1: DO1, databus_out2: DO2, in_info: INFO, config: Config) -> Self {
        assert!(in_info.is_mono());
        // Note: alignment_bytes returns bytes, but PayloadSize now conceptually works with Items.
        // Assuming min size logic needs to be adapted. 
        // For simplicity/safety, we request at least 1 item.
        let min = 1; 
        let preferred = config.prefer_samples_per_process;

        Self {
            consumer: ConsumerHandle::new(databus_in, PayloadSize::new(min, preferred)),
            producer1: ProducerHandle::new(databus_out1, PayloadSize::new(min, preferred)),
            producer2: ProducerHandle::new(databus_out2, PayloadSize::new(min, preferred)),
            info: in_info,
            config,
        }
    }
}

impl<DI, DO1, DO2, INFO, T> Node for ChannelSplitter2<DI, DO1, DO2, INFO, T>
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
    INFO: Info,
    T: Copy + 'static,
{
    type Error = ();

    async fn init(&mut self) -> Result<(), Self::Error> {
        // No dynamic initialization needed for this node
        Ok(())
    }

    async fn run(&mut self) -> Result<(), Self::Error> {
        loop {
            let payload_size = self.config.prefer_samples_per_process as usize;
            
            // Acquire read payload (T is inferred)
            let read_payload = self.consumer.acquire_read(payload_size).await;
            let actual_len = read_payload.len();

            // Acquire write payloads
            // Note: In a real circular buffer, acquiring two writes sequentially 
            // might lead to deadlock if the graph has cycles or backpressure. 
            // Be aware of this simple implementation.
            let mut write_payload1 = self.producer1.acquire_write(actual_len, true).await;
            let mut write_payload2 = self.producer2.acquire_write(actual_len, true).await;

            // Copy data (requires T: Copy)
            write_payload1[..actual_len].copy_from_slice(&read_payload[..actual_len]);
            write_payload2[..actual_len].copy_from_slice(&read_payload[..actual_len]);

            write_payload1.set_valid_length(actual_len);
            write_payload2.set_valid_length(actual_len);
            
            write_payload1.set_position(read_payload.position());
            write_payload2.set_position(read_payload.position());
        }
    }
}
