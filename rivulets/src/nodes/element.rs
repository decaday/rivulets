use rivulets_driver::element::{Element, ProcessStatus};
use rivulets_driver::format::Format;
use rivulets_driver::port::{InPort, OutPort, InPlacePort};
use rivulets_driver::databus::{Consumer, DatabusRef, Producer, Transformer};
use rivulets_driver::node::Node;

use crate::databus::{ConsumerHandle, ProducerHandle, TransformerHandle};

/// A generic node that wraps any `Element`, connecting it to one input and one output.
/// This is the most common type of node, used for processing/transformation tasks.
pub struct ElementNode<E, DI, DO, F>
where
    E: Element<Format=F>,
    DI: DatabusRef,
    DO: DatabusRef,
    ConsumerHandle<DI::Databus, DI>: Consumer<Item = E::InputItem>,
    ProducerHandle<DO::Databus, DO>: Producer<Item = E::OutputItem>,
    TransformerHandle<DO::Databus, DO>: Transformer<Item = E::OutputItem>,
    F: Format,
{
    element: E,
    databus_in: DI,
    databus_out: DO,
    in_port: InPort<ConsumerHandle<DI::Databus, DI>>,
    out_port: OutPort<ProducerHandle<DO::Databus, DO>>,
    in_place_port:  InPlacePort<TransformerHandle<DO::Databus, DO>>,
    _format: Option<F>,
}

impl<E, DI, DO, F> ElementNode<E, DI, DO, F>
where
    E: Element<Format=F>,
    DI: DatabusRef,
    DO: DatabusRef,
    ConsumerHandle<DI::Databus, DI>: Consumer<Item = E::InputItem>,
    ProducerHandle<DO::Databus, DO>: Producer<Item = E::OutputItem>,
    TransformerHandle<DO::Databus, DO>: Transformer<Item = E::OutputItem>,
    F: Format,
{
    pub fn new(element: E, databus_in: DI, databus_out: DO) -> Self {
        Self {
            element,
            databus_in,
            databus_out,
            in_port: InPort::None,
            out_port: OutPort::None,
            in_place_port: InPlacePort::None,
            _format: None,
        }
    }
}

impl<E, DI, DO, F> Node for ElementNode<E, DI, DO, F>
where
    E: Element<Format=F>,
    DI: DatabusRef,
    DO: DatabusRef,
    ConsumerHandle<DI::Databus, DI>: Consumer<Item = E::InputItem>,
    ProducerHandle<DO::Databus, DO>: Producer<Item = E::OutputItem>,
    TransformerHandle<DO::Databus, DO>: Transformer<Item = E::OutputItem>,
    F: Format,
{
    type Error = E::Error;

    async fn init(&mut self) -> Result<(), Self::Error> {
        self.element.initialize().await?;
        let reqs = self.element.get_port_requirements();
        if let Some(payload_size) = reqs.in_ {
            self.in_port = ConsumerHandle::new(self.databus_in.clone(), payload_size).in_port()
        }

        if let Some(payload_size) = reqs.out {
            self.out_port = ProducerHandle::new(self.databus_out.clone(), payload_size).out_port()
        }

        if let Some(payload_size) = reqs.in_place {
            self.in_place_port = TransformerHandle::new(self.databus_out.clone(), payload_size).in_place_port()
        }
        Ok(())
    }

    async fn run(&mut self) -> Result<(), Self::Error> {
        loop {
            let status = self.element.process(
                &self.in_port,
                &mut self.out_port,
                &mut self.in_place_port,
            ).await?;

            if status == ProcessStatus::Eof {
                break;
            }
        }
        Ok(())
    }
}
