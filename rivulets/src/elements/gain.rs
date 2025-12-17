use core::ops::MulAssign;

use rivulets_driver::databus::{Consumer, Producer, Transformer};
use rivulets_driver::element::{Element, ElementType, ProcessResult, ProcessStatus};
use rivulets_driver::format::Format;
use rivulets_driver::port::{InPlacePort, InPort, OutPort, PayloadSize, PortRequirements};

pub use crate::StandardConfig as Config;

pub struct Gain<F, T> {
    factor: T,
    format: F,
    config: Config,
}

impl<F, T> Gain<F, T>
where
    F: Format,
    T: Copy + 'static + MulAssign,
{
    pub fn new(factor: T, format: F, config: Config) -> Self {
        Self {
            factor,
            format,
            config,
        }
    }

    pub fn set_factor(&mut self, factor: T) {
        self.factor = factor;
    }
}

impl<F, T> Element for Gain<F, T>
where
    F: Format,
    T: Copy + 'static + MulAssign,
{
    type Format = F;
    type InputItem = T;
    type OutputItem = T;
    type Error = ();
    const TYPE: ElementType = ElementType::InPlaceTransformer;

    fn get_in_format(&self) -> Option<Self::Format> {
        Some(self.format)
    }

    fn get_out_format(&self) -> Option<Self::Format> {
        Some(self.format)
    }

    fn get_port_requirements(&self) -> PortRequirements {
        PortRequirements::new_in_place(PayloadSize {
            min: 1,
            preferred: self.config.prefer_items_per_process,
            exact: false,
        })
    }

    fn available(&self) -> u32 {
        u32::MAX
    }

    async fn process<C, P, Tr>(
        &mut self,
        _in_port: &InPort<C>,
        _out_port: &mut OutPort<P>,
        in_place_port: &mut InPlacePort<Tr>,
    ) -> ProcessResult<Self::Error>
    where
        C: Consumer<Item = Self::InputItem>,
        P: Producer<Item = Self::OutputItem>,
        Tr: Transformer<Item = Self::OutputItem>,
    {
        // Safe access to the transformer via pattern matching, 
        // as helper methods might vary in driver versions.
        let transformer = match in_place_port {
            InPlacePort::Transformer(t) => t,
            _ => panic!("Gain element connected to invalid port"),
        };

        let len = self.config.prefer_items_per_process as usize;
        let mut payload = transformer.acquire_transform(len).await;

        for sample in payload.iter_mut() {
            *sample *= self.factor;
        }

        payload.commit_all();

        Ok(ProcessStatus::Fine)
    }
}
