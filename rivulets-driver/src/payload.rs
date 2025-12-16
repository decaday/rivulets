use core::ops::{Deref, DerefMut};
use crate::databus::{Consumer, Producer, Transformer};

/// Metadata for payload data, describing stream properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Metadata {
    /// Position of this payload in a sequence.
    pub position: Position,
}

impl Metadata {
    pub fn new(position: Position) -> Self {
        Self { position }
    }
}

/// Position of a payload within a data sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Position {
    /// A single, complete payload (not part of a sequence).
    Single,
    /// The first payload in a sequence.
    First,
    /// The last payload in a sequence.
    Last,
    /// A middle payload in a sequence.
    #[default]
    Middle,
}

// --- Read Payload ---

/// A RAII guard representing a readable buffer acquired from a `Consumer`.
///
/// The `data` slice contains only valid data provided by the Databus.
/// Dropping this guard automatically releases the buffer back to the `Consumer`.
pub struct ReadPayload<'a, C: Consumer> {
    pub data: &'a [C::Item],
    pub metadata: Metadata,
    consumed_len: usize,
    consumer: &'a C,
}

impl<'a, C: Consumer> ReadPayload<'a, C> {
    pub fn new(data: &'a [C::Item], metadata: Metadata, consumer: &'a C) -> Self {
        Self {
            data,
            metadata,
            consumed_len: 0,
            consumer,
        }
    }

    /// Marks the specified amount of data as consumed.
    pub fn commit(&mut self, len: usize) {
        self.consumed_len = len.min(self.data.len());
    }

    pub fn commit_all(&mut self) {
        self.consumed_len = self.data.len();
    }

    pub fn position(&self) -> Position {
        self.metadata.position
    }
}

impl<'a, C: Consumer> Deref for ReadPayload<'a, C> {
    type Target = [C::Item];
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a, C: Consumer> Drop for ReadPayload<'a, C> {
    fn drop(&mut self) {
        self.consumer.release_read(self.metadata, self.consumed_len);
    }
}

// --- Write Payload ---

/// A RAII guard representing a writable buffer acquired from a `Producer`.
///
/// The `data` slice represents the full available capacity.
/// Dropping this guard automatically commits the written data.
pub struct WritePayload<'a, P: Producer> {
    data: &'a mut [P::Item],
    pub metadata: Metadata,
    written_len: usize,
    producer: &'a P,
}

impl<'a, P: Producer> WritePayload<'a, P> {
    pub fn new(data: &'a mut [P::Item], producer: &'a P) -> Self {
        Self {
            data,
            metadata: Metadata::default(),
            written_len: 0,
            producer,
        }
    }

    /// Commits the specified amount of written data.
    pub fn commit(&mut self, len: usize) {
        self.written_len = len.min(self.data.len());
    }

    pub fn commit_all(&mut self) {
        self.written_len = self.data.len();
    }

    /// Sets the position of this payload within a sequence.
    pub fn set_position(&mut self, position: Position) {
        self.metadata.position = position;
    }
}

impl<'a, P: Producer> Deref for WritePayload<'a, P> {
    type Target = [P::Item];
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a, P: Producer> DerefMut for WritePayload<'a, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

impl<'a, P: Producer> Drop for WritePayload<'a, P> {
    fn drop(&mut self) {
        self.producer.release_write(self.metadata, self.written_len);
    }
}

// --- Transform Payload ---

/// A RAII guard representing a buffer for in-place transformation.
///
/// The `data` slice contains the input data to be transformed.
/// Expansion of data (output > input) is not supported.
pub struct TransformPayload<'a, T: Transformer> {
    data: &'a mut [T::Item],
    pub metadata: Metadata,
    transformed_len: usize,
    transformer: &'a T,
}

impl<'a, T: Transformer> TransformPayload<'a, T> {
    pub fn new(data: &'a mut [T::Item], metadata: Metadata, transformer: &'a T) -> Self {
        Self {
            data,
            metadata,
            transformed_len: 0,
            transformer,
        }
    }

    /// Commits the specified amount of transformed data.
    /// This length must not exceed the input length.
    pub fn commit(&mut self, len: usize) {
        self.transformed_len = len.min(self.data.len());
    }

    pub fn commit_all(&mut self) {
        self.transformed_len = self.data.len();
    }
}

impl<'a, T: Transformer> Deref for TransformPayload<'a, T> {
    type Target = [T::Item];
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a, T: Transformer> DerefMut for TransformPayload<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

impl<'a, T: Transformer> Drop for TransformPayload<'a, T> {
    fn drop(&mut self) {
        self.transformer.release_transform(self.metadata, self.transformed_len);
    }
}