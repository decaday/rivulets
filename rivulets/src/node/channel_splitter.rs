use rivulets_driver::info::Info;
use rivulets_driver::payload;
use rivulets_driver::port::PayloadSize;
use rivulets_driver::databus::{self, Consumer, Databus, DatabusRef, Producer, Transformer};
use rivulets_driver::node::Node;

use crate::databus::{ConsumerHandle, ProducerHandle, TransformerHandle};

pub struct Config {
    perfer_samples_per_process: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self { 
            perfer_samples_per_process: 64, 
        }
    }
}

pub struct ChannelSplitter2<DI, DO1, DO2, INFO>
where
    DI: DatabusRef,
    DO1: DatabusRef,
    DO2: DatabusRef,
    ConsumerHandle<DI::Databus, DI>: Consumer,
    ProducerHandle<DO1::Databus, DO1>: Producer,
    ProducerHandle<DO2::Databus, DO2>: Producer,
    INFO: Info// <CHNNAL_NUM=1>
{
    consumer: ConsumerHandle<DI::Databus, DI>,
    producer1: ProducerHandle<DO1::Databus, DO1>,
    producer2: ProducerHandle<DO2::Databus, DO2>,
    info: INFO,
    config: Config,
}

impl<DI, DO1, DO2, INFO> ChannelSplitter2<DI, DO1, DO2, INFO>
where
    DI: DatabusRef,
    DO1: DatabusRef,
    DO2: DatabusRef,
    ConsumerHandle<DI::Databus, DI>: Consumer,
    ProducerHandle<DO1::Databus, DO1>: Producer,
    ProducerHandle<DO2::Databus, DO2>: Producer,
    INFO: Info// <CHNNAL_NUM=1>
{
    pub fn new(databus_in: DI, databus_out1: DO1, databus_out2: DO2, in_info: INFO, config: Config) -> Self {
        assert!(in_info.is_mono());
        assert!(config.perfer_samples_per_process % 2 == 0);
        let min = (in_info.alignment_bytes() / 2) as u16;
        let preferred = config.perfer_samples_per_process;

        Self {
            consumer: ConsumerHandle::new(databus_in, PayloadSize::new(min, preferred)),
            producer1: ProducerHandle::new(databus_out1, PayloadSize::new(min/2, preferred/2)),
            producer2: ProducerHandle::new(databus_out2, PayloadSize::new(min/2, preferred/2)),
            info: in_info,
            config,
        }
    }
}

impl<DI, DO1, DO2, INFO> Node for ChannelSplitter2<DI, DO1, DO2, INFO>
where
    DI: DatabusRef,
    DO1: DatabusRef,
    DO2: DatabusRef,
    ConsumerHandle<DI::Databus, DI>: Consumer,
    ProducerHandle<DO1::Databus, DO1>: Producer,
    ProducerHandle<DO2::Databus, DO2>: Producer,
    INFO: Info// <CHNNAL_NUM=1>
{
    type Error = ();

    async fn init(&mut self) -> Result<(), Self::Error> {
        todo!()
    }

    async fn run(&mut self) -> Result<(), Self::Error> {
        loop {
            let payload_size = self.config.perfer_samples_per_process as usize;
            let read_payload = self.consumer.acquire_read(payload_size).await;

            let actual_len = read_payload.len();

            let mut write_payload1 = self.producer1.acquire_write(actual_len, true).await;
            let mut write_payload2 = self.producer2.acquire_write(actual_len, true).await;

            write_payload1[..actual_len].copy_from_slice(&read_payload[..actual_len]);
            write_payload2[..actual_len].copy_from_slice(&read_payload[..actual_len]);

            write_payload1.set_valid_length(actual_len);
            write_payload2.set_valid_length(actual_len);
            
            write_payload1.set_position(read_payload.position());
            write_payload2.set_position(read_payload.position());
        }
    }
}