use fundsp::{audionode::AudioNode, hacker::Frame, combinator::An};
use fundsp::prelude::{U1, U0};

use rivulets_driver::element::ElementType;
use rivulets_driver::{
    databus::{Consumer, Producer, Transformer},
    element::{Element, ProcessResult, ProcessStatus::Fine},
    format::Format,
    port::{InPlacePort, InPort, OutPort, PayloadSize, PortRequirements},
};

use super::buffer::{SplitBuffer, SplitSourceBuffer, SplitSinkBuffer};

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

pub struct FundspElement<U, F>
where
    U: AudioNode<Inputs = U1, Outputs = U1>,
    F: Format,
{
    pub node: U,
    in_format: F,
    out_format: F,
    config: Config,
}

impl<U, F> FundspElement<U, F>
where
    U: AudioNode<Inputs = U1, Outputs = U1>,
    F: Format,
{
    pub fn new(node: U, in_format: F, out_format: F, config: Config) -> Self {
        assert!(in_format.valid());
        assert!(in_format.is_mono());
        assert!(out_format.valid());

        Self {
            node,
            in_format,
            out_format,
            config,
        }
    }

    pub fn from_an(an: An<U>, in_format: F, out_format: F, config: Config) -> Self {
        assert!(in_format.valid());
        assert!(in_format.is_mono());
        assert!(out_format.valid());

        Self {
            node: an.0,
            in_format,
            out_format,
            config,
        }
    }
}

impl<U, F> Element for FundspElement<U, F>
where
    U: AudioNode<Inputs = U1, Outputs = U1>,
    F: Format,
{
    type Format = F;
    type InputItem = f32;
    type OutputItem = f32;
    const TYPE: ElementType = ElementType::Processor;
    type Error = ();

    fn get_in_format(&self) -> Option<Self::Format> {
        Some(self.in_format)
    }

    fn get_out_format(&self) -> Option<Self::Format> {
        Some(self.out_format)
    }

    fn available(&self) -> u32 {
        todo!()
    }

    fn get_port_requirements(&self) -> PortRequirements {
        let min = 1;
        PortRequirements {
            in_: Some(PayloadSize::new(min, self.config.prefer_items_per_process, false)),
            out: Some(PayloadSize::new(min, self.config.prefer_items_per_process, false)),
            in_place: None,
        }
    }

    async fn initialize(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn process<C, P, T>(
        &mut self,
        in_port: &InPort<C>,
        out_port: &mut OutPort<P>,
        _in_place_port: &mut InPlacePort<T>,
    ) -> ProcessResult<Self::Error>
    where
        C: Consumer<Item = Self::InputItem>,
        P: Producer<Item = Self::OutputItem>,
        T: Transformer<Item = Self::OutputItem>,
    {
        let consumer = in_port.consumer_ref();
        let producer = out_port.producer_ref();

        let mut write_payload = producer
            .acquire_write(self.config.prefer_items_per_process as usize)
            .await;
        
        let mut read_payload = consumer.acquire_read(write_payload.len()).await;
        
        let process_len = read_payload.len();
        let write_slice = &mut write_payload[..process_len];

        {
            let mut split = SplitBuffer::new(&read_payload, write_slice);

            split.scalar_pairs().for_each(|(i, o)| {
                *o = self.node.tick(&Frame::from([*i]))[0];
            });

            if let Some((samples, input_buffer, mut output_buffer)) = split.simd_parts() {
                self.node.process(samples, &input_buffer, &mut output_buffer);
            }
        }
        
        read_payload.commit(process_len);
        write_payload.commit(process_len);
        write_payload.set_position(read_payload.position());

        Ok(Fine)
    }

    async fn reset(&mut self) -> Result<(), Self::Error> {
        self.node.reset();
        Ok(())
    }
}

pub struct FundspSourceElement<U, F>
where
    U: AudioNode<Inputs = U0, Outputs = U1>,
    F: Format,
{
    pub node: U,
    out_format: F,
    config: Config,
}

impl<U, F> FundspSourceElement<U, F>
where
    U: AudioNode<Inputs = U0, Outputs = U1>,
    F: Format,
{
    pub fn new(node: U, out_format: F, config: Config) -> Self {
        assert!(out_format.valid());
        assert!(out_format.is_mono());

        Self {
            node,
            out_format,
            config,
        }
    }

    pub fn from_an(an: An<U>, out_format: F, config: Config) -> Self {
        Self::new(an.0, out_format, config)
    }
}

impl<U, F> Element for FundspSourceElement<U, F>
where
    U: AudioNode<Inputs = U0, Outputs = U1>,
    F: Format,
{
    type Format = F;
    type InputItem = (); 
    type OutputItem = f32;
    const TYPE: ElementType = ElementType::Source;
    type Error = ();

    fn get_in_format(&self) -> Option<Self::Format> {
        None
    }

    fn get_out_format(&self) -> Option<Self::Format> {
        Some(self.out_format)
    }

    fn available(&self) -> u32 {
        u32::MAX
    }

    fn get_port_requirements(&self) -> PortRequirements {
        let min = 1;
        PortRequirements {
            in_: None,
            out: Some(PayloadSize::new(min, self.config.prefer_items_per_process, false)),
            in_place: None,
        }
    }

    async fn initialize(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn process<C, P, T>(
        &mut self,
        _in_port: &InPort<C>,
        out_port: &mut OutPort<P>,
        _in_place_port: &mut InPlacePort<T>,
    ) -> ProcessResult<Self::Error>
    where
        C: Consumer<Item = Self::InputItem>,
        P: Producer<Item = Self::OutputItem>,
        T: Transformer<Item = Self::OutputItem>,
    {
        let producer = out_port.producer_ref();

        let mut write_payload = producer
            .acquire_write(self.config.prefer_items_per_process as usize)
            .await;

        let mut split = SplitSourceBuffer::new(&mut write_payload);

        let empty_frame = Frame::default();
        split.scalar_parts().for_each(|s| {
            *s = self.node.tick(&empty_frame)[0];
        });

        if let Some((samples, input_buffer, mut output_buffer)) = split.simd_parts() {
            self.node.process(samples, &input_buffer, &mut output_buffer);
        }

        write_payload.commit_all();
        
        Ok(Fine)
    }

    async fn reset(&mut self) -> Result<(), Self::Error> {
        self.node.reset();
        Ok(())
    }
}

pub struct FundspSinkElement<U, F>
where
    U: AudioNode<Inputs = U1, Outputs = U0>,
    F: Format,
{
    pub node: U,
    in_format: F,
    config: Config,
}

impl<U, F> FundspSinkElement<U, F>
where
    U: AudioNode<Inputs = U1, Outputs = U0>,
    F: Format,
{
    pub fn new(node: U, in_format: F, config: Config) -> Self {
        assert!(in_format.valid());
        assert!(in_format.is_mono());

        Self {
            node,
            in_format,
            config,
        }
    }

    pub fn from_an(an: An<U>, in_format: F, config: Config) -> Self {
        Self::new(an.0, in_format, config)
    }
}

impl<U, F> Element for FundspSinkElement<U, F>
where
    U: AudioNode<Inputs = U1, Outputs = U0>,
    F: Format,
{
    type Format = F;
    type InputItem = f32;
    type OutputItem = ();
    const TYPE: ElementType = ElementType::Sink;
    type Error = ();

    fn get_in_format(&self) -> Option<Self::Format> {
        Some(self.in_format)
    }

    fn get_out_format(&self) -> Option<Self::Format> {
        None
    }

    fn available(&self) -> u32 {
        u32::MAX
    }

    fn get_port_requirements(&self) -> PortRequirements {
        let min = 1;
        PortRequirements {
            in_: Some(PayloadSize::new(min, self.config.prefer_items_per_process, false)),
            out: None,
            in_place: None,
        }
    }

    async fn initialize(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn process<C, P, T>(
        &mut self,
        in_port: &InPort<C>,
        _out_port: &mut OutPort<P>,
        _in_place_port: &mut InPlacePort<T>,
    ) -> ProcessResult<Self::Error>
    where
        C: Consumer<Item = Self::InputItem>,
        P: Producer<Item = Self::OutputItem>,
        T: Transformer<Item = Self::OutputItem>,
    {
        let consumer = in_port.consumer_ref();

        let mut read_payload = consumer
            .acquire_read(self.config.prefer_items_per_process as usize)
            .await;

        let mut split = SplitSinkBuffer::new(&read_payload);

        split.scalar_parts().for_each(|s| {
            self.node.tick(&Frame::from([*s]));
        });

        if let Some((samples, input_buffer, mut output_buffer)) = split.simd_parts() {
            self.node.process(samples, &input_buffer, &mut output_buffer);
        }

        read_payload.commit_all();
        Ok(Fine)
    }

    async fn reset(&mut self) -> Result<(), Self::Error> {
        self.node.reset();
        Ok(())
    }
}
