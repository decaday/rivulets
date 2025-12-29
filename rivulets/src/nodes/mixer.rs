use core::ops::Add;

use rivulets_driver::databus::{Consumer, Databus, DatabusRef, Producer};
use rivulets_driver::format::Format;
use rivulets_driver::node::Node;
use rivulets_driver::port::PayloadSize;

use crate::databus::{ConsumerHandle, ProducerHandle};

pub struct Config {
    pub prefer_items_per_process: u16,
    pub equicvalent_inputs: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            prefer_items_per_process: 64,
            equicvalent_inputs: true,
        }
    }
}

/// A node that mixes (sums) two input streams into one output stream.
///
/// It requires inputs to be synchronized. In the current `Slot` implementation,
/// it also requires inputs to provide chunks of identical size.
pub struct Mixer2<D1, D2, DO, F, T>
where
    D1: DatabusRef,
    D2: DatabusRef,
    DO: DatabusRef,
    D1::Databus: Databus<Item = T>,
    D2::Databus: Databus<Item = T>,
    DO::Databus: Databus<Item = T>,
    ConsumerHandle<D1::Databus, D1>: Consumer<Item = T>,
    ConsumerHandle<D2::Databus, D2>: Consumer<Item = T>,
    ProducerHandle<DO::Databus, DO>: Producer<Item = T>,
    F: Format,
    T: Copy + Add<Output = T> + 'static,
{
    consumer_1: ConsumerHandle<D1::Databus, D1>,
    consumer_2: ConsumerHandle<D2::Databus, D2>,
    producer: ProducerHandle<DO::Databus, DO>,
    _format: F,
    config: Config,
}

impl<D1, D2, DO, F, T> Mixer2<D1, D2, DO, F, T>
where
    D1: DatabusRef,
    D2: DatabusRef,
    DO: DatabusRef,
    D1::Databus: Databus<Item = T>,
    D2::Databus: Databus<Item = T>,
    DO::Databus: Databus<Item = T>,
    ConsumerHandle<D1::Databus, D1>: Consumer<Item = T>,
    ConsumerHandle<D2::Databus, D2>: Consumer<Item = T>,
    ProducerHandle<DO::Databus, DO>: Producer<Item = T>,
    F: Format,
    T: Copy + Add<Output = T> + 'static,
{
    pub fn new(
        databus_in_1: D1,
        databus_in_2: D2,
        databus_out: DO,
        format: F,
        config: Config,
    ) -> Self {
        let min = 1;
        let preferred = config.prefer_items_per_process;
        let payload_size = PayloadSize::new(min, preferred);
        let strict_alloc = false;
        let consume_all = config.equicvalent_inputs;

        Self {
            consumer_1: ConsumerHandle::new(databus_in_1.clone(), payload_size, consume_all),
            consumer_2: ConsumerHandle::new(databus_in_2.clone(), payload_size, consume_all),
            producer: ProducerHandle::new(databus_out, payload_size, strict_alloc),
            _format: format,
            config,
        }
    }
}

impl<D1, D2, DO, F, T> Node for Mixer2<D1, D2, DO, F, T>
where
    D1: DatabusRef,
    D2: DatabusRef,
    DO: DatabusRef,
    D1::Databus: Databus<Item = T>,
    D2::Databus: Databus<Item = T>,
    DO::Databus: Databus<Item = T>,
    ConsumerHandle<D1::Databus, D1>: Consumer<Item = T>,
    ConsumerHandle<D2::Databus, D2>: Consumer<Item = T>,
    ProducerHandle<DO::Databus, DO>: Producer<Item = T>,
    F: Format,
    T: Copy + Add<Output = T> + 'static,
{
    type Error = ();

    async fn init(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn run(&mut self) -> Result<(), Self::Error> {
        loop {
            let len = self.config.prefer_items_per_process as usize;

            let mut write_payload = self.producer.acquire_write(len).await;
            let payload_len = write_payload.len();
            
            let mut read_payload_1 = self.consumer_1.acquire_read(payload_len).await;
            let mut read_payload_2 = self.consumer_2.acquire_read(payload_len).await;

            let process_len = if self.config.equicvalent_inputs {
                assert_eq!(read_payload_1.len(), read_payload_2.len());
                read_payload_1.len()
            } else {
                read_payload_1.len().min(read_payload_2.len())
            };

            for ((&in1, &in2), out) in read_payload_1[..process_len].iter()
                .zip(read_payload_2[..process_len].iter())
                .zip(write_payload[..process_len].iter_mut()) 
            {
                *out = in1 + in2;
            }

            read_payload_1.commit(process_len);
            read_payload_2.commit(process_len);
            write_payload.commit(process_len);
            
            write_payload.set_position(read_payload_1.position());
        }
    }
}