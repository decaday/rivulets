#[allow(async_fn_in_trait)]
pub trait Node {
    type Error;

    /// Initializes the node and its underlying element.
    ///
    /// This is where `Format` negotiation happens. The pipeline builder calls this
    /// for each node sequentially to ensure compatibility before running the pipeline.
    async fn init(&mut self) -> Result<(), Self::Error>;

    /// The main execution loop for the node.
    /// This async function runs indefinitely, processing data until the stream ends or an error occurs.
    async fn run(&mut self) -> Result<(), Self::Error>;
}