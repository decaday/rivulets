use fundsp::buffer::{BufferMut, BufferRef};
use fundsp::{audionode::AudioNode, hacker::Frame};
use fundsp::prelude::{U1};
use rivulets_driver::element::ElementType;
use rivulets_driver::{
    databus::{Consumer, Producer, Transformer},
    element::{Element, ProcessResult, ProcessStatus::Fine},
    format::Format,
    port::{InPlacePort, InPort, OutPort, PayloadSize, PortRequirements},
};

use super::buffer::SplitBuffer;

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

        // 2. Use SplitBuffer to handle alignment and split into scalar/SIMD parts.
        // `read_payload` derefs to &[f32] and `write_payload` derefs to &mut [f32].
        let split = SplitBuffer::new(&read_payload, &mut write_payload);

        // 3. Process the unaligned head (scalar).
        let (head_in, head_out) = split.head;
        head_in.iter().zip(head_out.iter_mut()).for_each(|(i, o)| {
            *o = self.node.tick(&Frame::from([*i]))[0];
        });

        // 4. Process the aligned body (SIMD block).
        let (body_in, body_out) = split.body;
        if !body_in.is_empty() {
            
            let input_buffer = BufferRef::new(body_in);
            let mut output_buffer = BufferMut::new(body_out);
            
            // Calculate total samples in the body part.
            // Note: fundsp `process` size argument is usually in samples, not SIMD vectors.
            // However, BufferRef length is checked internally.
            // fundsp::AudioNode::process signature: (size: usize, input, output).
            // `size` is the number of samples to process.
            let samples = body_in.len() * fundsp::SIMD_LEN; // F32x len
            
            self.node.process(samples, &input_buffer, &mut output_buffer);
        }

        // 5. Process the unaligned tail (scalar).
        let (tail_in, tail_out) = split.tail;
        tail_in.iter().zip(tail_out.iter_mut()).for_each(|(i, o)| {
            *o = self.node.tick(&Frame::from([*i]))[0];
        });

        write_payload.set_valid_length(write_payload.len());
        write_payload.set_position(read_payload.position());

        Ok(Fine)
    }

    async fn reset(&mut self) -> Result<(), Self::Error> {
        self.node.reset();
        Ok(())
    }
}
