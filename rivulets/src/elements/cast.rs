use core::marker::PhantomData;

use num_traits::AsPrimitive;

use rivulets_driver::databus::{Consumer, Producer, Transformer};
use rivulets_driver::element::{Element, ElementType, ProcessResult, ProcessStatus};
use rivulets_driver::format::Format;
use rivulets_driver::port::{InPlacePort, InPort, OutPort, PayloadSize, PortRequirements};

pub use crate::StandardConfig as Config;

/// An element that converts data items from type `I` to type `O`.
pub struct Cast<I, O, F> {
    format: F,
    config: Config,
    _marker: PhantomData<(I, O)>,
}

impl<I, O, F> Cast<I, O, F>
where
    I: Copy + 'static + AsPrimitive<O>,
    O: Copy + 'static,
    F: Format,
{
    pub fn new(format: F, config: Config) -> Self {
        Self {
            format,
            config,
            _marker: PhantomData,
        }
    }
}

impl<I, O, F> Element for Cast<I, O, F>
where
    I: Copy + 'static + AsPrimitive<O>,
    O: Copy + 'static,
    F: Format,
{
    type Format = F;
    type InputItem = I;
    type OutputItem = O;
    type Error = ();
    const TYPE: ElementType = ElementType::Processor;

    fn get_in_format(&self) -> Option<Self::Format> {
        Some(self.format)
    }

    fn get_out_format(&self) -> Option<Self::Format> {
        Some(self.format)
    }

    fn get_port_requirements(&self) -> PortRequirements {
        PortRequirements::new_payload_to_payload(
            PayloadSize {
                min: 1,
                preferred: self.config.prefer_items_per_process,
                exact: false,
            },
            PayloadSize {
                min: 1,
                preferred: self.config.prefer_items_per_process,
                exact: false,
            },
        )
    }

    fn available(&self) -> u32 {
        u32::MAX
    }

    async fn process<C, P, Tr>(
        &mut self,
        in_port: &InPort<C>,
        out_port: &mut OutPort<P>,
        _in_place_port: &mut InPlacePort<Tr>,
    ) -> ProcessResult<Self::Error>
    where
        C: Consumer<Item = Self::InputItem>,
        P: Producer<Item = Self::OutputItem>,
        Tr: Transformer<Item = Self::OutputItem>,
    {
        let consumer = in_port.consumer_ref();
        let producer = out_port.producer_ref();

        let len = self.config.prefer_items_per_process as usize;
        
        let mut write_payload = producer.acquire_write(len).await;

        let mut read_payload = consumer.acquire_read(write_payload.len()).await;
        let actual_len = read_payload.len();
        
        for (in_item, out_item) in read_payload.iter().zip(write_payload[..actual_len].iter_mut()) {
            *out_item = in_item.as_();
        }

        read_payload.commit_all();
        write_payload.commit(actual_len);
        write_payload.set_position(read_payload.position());

        Ok(ProcessStatus::Fine)
    }
}
