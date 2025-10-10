use crate::databus::{Consumer, Producer, Transformer};
use crate::info::Info;
use crate::port::{InPlacePort, InPort, OutPort, PortRequirements};

/// The core trait for any data processing unit in the pipeline.
///
/// An `Element` can be a source (generator), a sink (writer),
/// or a transformation (filter). It processes data by receiving it from
/// an `InPort` and sending it to an `OutPort`.
#[allow(async_fn_in_trait)]
pub trait Element {
    /// The type of metadata associated with the data stream.
    type Info: Info;
    /// The error type for this element's operations.
    type Error;

    /// Returns the metadata expected at the input.
    /// Returns `None` if this is a source element.
    fn get_in_info(&self) -> Option<Self::Info>;

    /// Returns the metadata produced at the output.
    /// Returns `None` if this is a sink element.
    fn get_out_info(&self) -> Option<Self::Info>;

    fn get_port_requirements(&self) -> PortRequirements;

    /// Returns the amount of available data for processing.
    /// The unit (e.g., bytes, frames) depends on the element's nature.
    fn available(&self) -> u32;

    /// Initializes the element based on metadata from the upstream element.
    /// Returns its own port requirements upon successful initialization.
    async fn initialize(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Flushes any internal buffers of the element.
    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Resets the internal state and info of the element.
    async fn reset(&mut self) -> Result<(), Self::Error> {
        self.flush().await
    }

    /// The main asynchronous processing function.
    ///
    /// It takes an `InPort`, an `OutPort`, and an `InPlacePort` to perform one step of data
    /// processing, by acquiring and releasing payloads from the databus.
    async fn process<'a, C, P, T>(
        &mut self,
        in_port: &'a InPort<'a, C>,
        out_port: &'a mut OutPort<'a, P>,
        in_place_port: &'a mut InPlacePort<'a, T>,
    ) -> ProcessResult<Self::Error>
    where
        C: Consumer<'a>,
        P: Producer<'a>,
        T: Transformer<'a>;
}

/// Represents the status of a `process` call.
#[derive(Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    /// The processing step completed successfully, and more data may follow.
    Fine,
    /// The element has reached the end of its data stream.
    Eof,
}
pub use ProcessStatus::{Eof, Fine};

/// A convenient type alias for the result of a `process` call.
pub type ProcessResult<E> = Result<ProcessStatus, E>;