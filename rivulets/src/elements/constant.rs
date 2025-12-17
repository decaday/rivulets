use rivulets_driver::databus::{Consumer, Producer, Transformer};
use rivulets_driver::element::{Element, ElementType, ProcessResult, ProcessStatus};
use rivulets_driver::format::Format;
use rivulets_driver::port::{InPlacePort, InPort, OutPort, PayloadSize, PortRequirements};

pub use crate::StandardConfig as Config;

pub struct Constant<F, T> {
    value: T,
    format: F,
    config: Config,
}

impl<F, T> Constant<F, T>
where
    F: Format,
    T: Copy + 'static,
{
    pub fn new(value: T, format: F, config: Config) -> Self {
        Self {
            value,
            format,
            config,
        }
    }
}

impl<F, T> Element for Constant<F, T>
where
    F: Format,
    T: Copy + 'static,
{
    type Format = F;
    type InputItem = ();
    type OutputItem = T;
    type Error = ();
    const TYPE: ElementType = ElementType::Source;

    fn get_in_format(&self) -> Option<Self::Format> {
        None
    }

    fn get_out_format(&self) -> Option<Self::Format> {
        Some(self.format)
    }

    fn get_port_requirements(&self) -> PortRequirements {
        PortRequirements::source(
            PayloadSize::new(1, self.config.prefer_items_per_process),
            false,
        )
    }

    fn available(&self) -> u32 {
        u32::MAX
    }

    async fn process<C, P, Tr>(
        &mut self,
        _in_port: &InPort<C>,
        out_port: &mut OutPort<P>,
        _in_place_port: &mut InPlacePort<Tr>,
    ) -> ProcessResult<Self::Error>
    where
        C: Consumer<Item = Self::InputItem>,
        P: Producer<Item = Self::OutputItem>,
        Tr: Transformer<Item = Self::OutputItem>,
    {
        let producer = out_port.producer_ref();

        let len = self.config.prefer_items_per_process as usize;
        let mut payload = producer.acquire_write(len).await;

        payload.fill(self.value);

        payload.commit_all();

        Ok(ProcessStatus::Fine)
    }
}
