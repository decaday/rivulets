use fundsp::{audionode::AudioNode, hacker::Frame};
use fundsp::prelude::{U0, U1};
use rivulets_driver::element::ElementType;
use rivulets_driver::{databus::{Consumer, Producer, Transformer}, element::{Element, ProcessResult, ProcessStatus::Fine}, info::Info, port::{InPlacePort, InPort, OutPort, PayloadSize, PortRequirements}};
use super::buffer::BufferPayload;

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

impl Config {
    fn payload_size<I: Info>(&self, info: I) -> usize {
        (self.perfer_samples_per_process as usize) * (info.alignment_bytes() as usize)
    }
}

pub struct FanDspNodeElement<U, I>
where 
    U: AudioNode<Inputs = U1, Outputs = U1>,
    I: Info,
{
    pub node: U,
    in_info: I, 
    out_info: I,
    config: Config,
}

impl<U, I> FanDspNodeElement<U, I> 
where 
    U: AudioNode<Inputs = U1, Outputs = U1>,
    I: Info,
{
    pub fn new(node: U, in_info: I, out_info: I, config: Config) -> Self {
        assert!(in_info.vaild());
        assert!(in_info.is_f32());
        assert!(in_info.is_mono());

        assert!(out_info.vaild());
        assert!(out_info.is_f32());
        assert!(out_info.is_mono());
        
        Self {
            node,
            in_info,
            out_info,
            config,
        }
    }
}

impl<U, I> Element for FanDspNodeElement<U, I> 
where 
    U: AudioNode<Inputs = U1, Outputs = U1>,
    I: Info,
{
    type Info = I;
    const TYPE: ElementType = ElementType::Processor;
    type Error = ();

    fn get_in_info(&self) -> Option<Self::Info> {
        Some(self.in_info)
    }

    fn get_out_info(&self) -> Option<Self::Info> {
        Some(self.out_info)
    }

    fn available(&self) -> u32 {
        todo!()
    }

    fn get_port_requirements(&self) -> PortRequirements {
        let min = self.in_info.bytes_per_simple() as u16;
        PortRequirements {
            in_: Some(PayloadSize{ min, preferred: self.config.perfer_samples_per_process }),
            out: Some(PayloadSize{ min, preferred: self.config.perfer_samples_per_process }),
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
        C: Consumer,
        P: Producer,
        T: Transformer, 
    {
        let consumer = in_port.consumer_ref();
        let producer = out_port.producer_ref();
    

        let read_payload = consumer.acquire_read(self.config.payload_size(self.in_info)).await;
        let mut write_payload = producer.acquire_write(read_payload.len(), true).await;

        let mut buffers = BufferPayload::new(&read_payload, &mut write_payload);

        buffers.iter_mut().for_each(|(input, output)| {
            *output = self.node.tick(&Frame::from([*input]))[0];
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
