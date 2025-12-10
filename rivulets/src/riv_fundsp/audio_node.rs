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
        // Request minimum 1 item (sample).
        // The preferred size should be a multiple of SIMD width (e.g. 64) for best performance.
        let min = 1;
        PortRequirements {
            in_: Some(PayloadSize {
                min,
                preferred: self.config.prefer_items_per_process,
            }),
            out: Some(PayloadSize {
                min,
                preferred: self.config.prefer_items_per_process,
            }),
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
        // Constraint: We currently only support f32 processing for fundsp nodes.
        C: Consumer<Item = Self::InputItem>,
        P: Producer<Item = Self::OutputItem>,
        T: Transformer<Item = Self::OutputItem>,
    {
        let consumer = in_port.consumer_ref();
        let producer = out_port.producer_ref();

        // 1. Acquire generic f32 payloads.
        let read_payload = consumer
            .acquire_read(self.config.prefer_items_per_process as usize)
            .await;
        
        let mut write_payload = producer.acquire_write(read_payload.len(), true).await;

        // 2. Use SplitBuffer to handle alignment and SIMD processing.
        let mut split = SplitBuffer::new(&read_payload, &mut write_payload);

        // 3. Process unaligned scalar edges (Head and Tail).
        split.scalar_pairs().for_each(|(i, o)| {
            *o = self.node.tick(&Frame::from([*i]))[0];
        });

        // 4. Process aligned SIMD body.
        if let Some((samples, input_buffer, mut output_buffer)) = split.simd_parts() {
            self.node.process(samples, &input_buffer, &mut output_buffer);
        }

        write_payload.set_valid_length(write_payload.len());
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
        // Source currently assumes Mono output based on Format trait limits,
        // but can be extended later.
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
    // Source has no input data, use () as placeholder
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
        // Sources are infinite
        u32::MAX
    }

    fn get_port_requirements(&self) -> PortRequirements {
        let min = 1;
        PortRequirements {
            in_: None, // No input port needed
            out: Some(PayloadSize {
                min,
                preferred: self.config.prefer_items_per_process,
            }),
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

        // 1. Only acquire write payload
        let mut write_payload = producer
            .acquire_write(self.config.prefer_items_per_process as usize, true)
            .await;

        // 2. Handle SIMD Alignment for Output Only
        let mut split = SplitSourceBuffer::new(&mut write_payload);

        // 3. Process unaligned scalar edges.
        let empty_frame = Frame::default();
        split.scalar_parts().for_each(|s| {
            *s = self.node.tick(&empty_frame)[0];
        });

        // 4. Process aligned SIMD body.
        if let Some((samples, input_buffer, mut output_buffer)) = split.simd_parts() {
            self.node.process(samples, &input_buffer, &mut output_buffer);
        }

        write_payload.set_valid_length(write_payload.len());
        
        Ok(Fine)
    }

    async fn reset(&mut self) -> Result<(), Self::Error> {
        self.node.reset();
        Ok(())
    }
}

/// A sink element that consumes audio data into a FunDSP node with no outputs (U1 -> U0).
/// Useful for monitoring, analysis, or side-effects.
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
        // Sink currently assumes Mono input based on Format trait limits.
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
            in_: Some(PayloadSize {
                min,
                preferred: self.config.prefer_items_per_process,
            }),
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

        // 1. Only acquire read payload
        let read_payload = consumer
            .acquire_read(self.config.prefer_items_per_process as usize)
            .await;

        // 2. Handle SIMD Alignment for Input Only
        let mut split = SplitSinkBuffer::new(&read_payload);

        // 3. Process unaligned scalar edges.
        split.scalar_parts().for_each(|s| {
            self.node.tick(&Frame::from([*s]));
        });

        // 4. Process aligned SIMD body.
        if let Some((samples, input_buffer, mut output_buffer)) = split.simd_parts() {
            self.node.process(samples, &input_buffer, &mut output_buffer);
        }

        Ok(Fine)
    }

    async fn reset(&mut self) -> Result<(), Self::Error> {
        self.node.reset();
        Ok(())
    }
}
