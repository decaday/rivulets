use core::marker::PhantomData;

use rivulets_driver::databus::{Consumer, Producer, Transformer};
use rivulets_driver::element::{Element, ElementType, ProcessResult, ProcessStatus};
use rivulets_driver::format::Format;
use rivulets_driver::port::{InPlacePort, InPort, OutPort, PayloadSize, PortRequirements};

#[derive(Debug, Clone, Copy)]
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

/// An element that applies a function `op` to each data item in place.
pub struct Map<F, T, Op> {
    op: Op,
    format: F,
    config: Config,
    _marker: PhantomData<T>,
}

impl<F, T, Op> Map<F, T, Op>
where
    F: Format,
    T: Copy + 'static,
    Op: FnMut(T) -> T,
{
    pub fn new(op: Op, format: F, config: Config) -> Self {
        Self {
            op,
            format,
            config,
            _marker: PhantomData,
        }
    }
}

impl<F, T, Op> Element for Map<F, T, Op>
where
    F: Format,
    T: Copy + 'static,
    Op: FnMut(T) -> T,
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
        let transformer = match in_place_port {
            InPlacePort::Transformer(t) => t,
            _ => panic!("Map element connected to invalid port"),
        };

        let len = self.config.prefer_items_per_process as usize;
        let mut payload = transformer.acquire_transform(len).await;

        for sample in payload.iter_mut() {
            *sample = (self.op)(*sample);
        }

        payload.commit_all();

        Ok(ProcessStatus::Fine)
    }
}
