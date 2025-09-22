#![macro_use]

use core::cell::UnsafeCell;
use core::future::poll_fn;
use core::mem::MaybeUninit;
use core::task::Poll;

use embassy_sync::waitqueue::AtomicWaker;
use portable_atomic::{AtomicU8, Ordering};

use rivulets_driver::databus::{Consumer, Databus, Operation, Producer, Transformer};
use rivulets_driver::payload::{Metadata, ReadPayload, TransformPayload, WritePayload};
use rivulets_driver::port::PayloadSize;

use crate::storage::Storage;
#[cfg(feature = "alloc")]
use crate::storage::Heap;
use crate::storage::Array;

pub type StaticSlot<const N: usize> = Slot<Array<u8, N>>;
#[cfg(feature = "alloc")]
pub type HeapSlot = Slot<Heap<u8>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum State {
    /// The slot is empty and ready for a producer.
    Empty,
    /// A producer is currently writing to the buffer.
    Writing,
    /// The slot is full with raw data, ready for the next stage (transformer or consumer).
    Full,
    /// A transformer is currently modifying the buffer in-place.
    Transforming,
    /// The data has been transformed (or is ready for consumption directly), now ready ONLY for a consumer.
    Transformed,
    /// A consumer is currently reading from the buffer.
    Reading,
}

impl From<State> for u8 {
    fn from(state: State) -> Self {
        state as u8
    }
}

impl TryFrom<u8> for State {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            x if x == State::Empty as u8 => Ok(State::Empty),
            x if x == State::Reading as u8 => Ok(State::Reading),
            x if x == State::Writing as u8 => Ok(State::Writing),
            x if x == State::Full as u8 => Ok(State::Full),
            x if x == State::Transforming as u8 => Ok(State::Transforming),
            x if x == State::Transformed as u8 => Ok(State::Transformed),
            _ => Err(()),
        }
    }
}


/// A single-slot channel with a configurable state machine that correctly sequences
/// an optional in-place transformation between the producer and consumer.
pub struct Slot<S: Storage<Item = u8>> {
    // The storage is now held directly. The UnsafeCell for interior mutability
    // is handled within the Storage implementations (e.g., Owning, Array, Heap).
    storage: S,
    payload_metadata: UnsafeCell<Option<Metadata>>,
    state: AtomicU8,
    /// Waker for the producer task, which waits for the `Empty` state.
    producer_waker: AtomicWaker,
    /// Waker for the transformer task, which waits for the `Full` state.
    transformer_waker: AtomicWaker,
    /// Waker for the final consumer task, which waits for `Transformed`.
    consumer_waker: AtomicWaker,
    registered: [bool; Operation::COUNT],
}

// The Storage trait implementations (like Owning) are responsible for ensuring
// they are Sync if they are to be used across threads. The Slot itself can
// be Sync if the underlying storage S is Sync.
unsafe impl<S: Storage<Item = u8> + Sync> Sync for Slot<S> {}

impl<S: Storage<Item = u8>> Slot<S> {
    /// Creates a new Slot from a given storage.
    ///
    /// # Arguments
    ///
    /// * `storage`: The storage backend to be managed by the slot.
    pub fn new(storage: S) -> Self {
        Slot {
            storage,
            payload_metadata: UnsafeCell::new(None),
            state: AtomicU8::new(State::Empty as u8),
            producer_waker: AtomicWaker::new(),
            transformer_waker: AtomicWaker::new(),
            consumer_waker: AtomicWaker::new(),
            registered: [false; Operation::COUNT],
        }
    }

    /// Returns a copy of the current payload metadata, if any.
    pub fn get_current_metadata(&self) -> Option<Metadata> {
        unsafe { (*self.payload_metadata.get()).clone() }
    }
}

/// Methods for creating Slots with heap-allocated buffers.
#[cfg(feature = "alloc")]
impl Slot<Heap<u8>> {
    /// Creates a new Slot with a heap-allocated buffer of a specified capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity`: The size of the buffer to allocate.
    pub fn new_heap(capacity: usize) -> Self {
        Self::new(Heap::new(capacity))
    }
}

/// Methods for creating Slots with statically-allocated array buffers.
impl<const N: usize> Slot<Array<u8, N>> {
    /// Creates a new Slot with a statically-allocated array buffer of size N.
    /// The buffer is uninitialized.
    pub fn new_static() -> Self {
        // The `Owning` wrapper inside `Array` provides the necessary UnsafeCell.
        Self::new(Array::from([MaybeUninit::uninit(); N]))
    }
}

impl<S: Storage<Item = u8>> Databus for Slot<S> {
    fn register(&mut self, operation: Operation, payload_size: PayloadSize) {
        let len = self.storage.len();

        if payload_size.preferred as usize > len {
            warn!("Slot buffer({}) is smaller than preferred size({})", len, payload_size.preferred);
        }

        if core::mem::replace(&mut self.registered[operation as usize], true) {
            panic!("{:?}(er) already registered", operation);
        }
    }
}

impl<'b, S: Storage<Item = u8> + 'b> Producer<'b> for Slot<S> {
    async fn acquire_write(&'b self) -> WritePayload<'b, Self> {
        debug_assert!(self.registered[Operation::Produce as usize], "acquire_write called on a Slot configured without a producer");
        poll_fn(|cx| {
            // Attempt to transition from Empty to Writing.
            if self.state.compare_exchange(
                State::Empty as u8, State::Writing as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                // On success, grant write access to the entire buffer.
                // The `Storage` trait's `slice_mut` can be called on `&self`
                // because the underlying implementation uses UnsafeCell.
                let buffer = unsafe { self.storage.slice_mut(0..self.storage.len()) };
                // Transmute from `MaybeUninit<u8>` to `u8` is safe here for writing.
                let buffer_ready_for_write = unsafe { &mut *(buffer as *mut [MaybeUninit<u8>] as *mut [u8]) };
                Poll::Ready(WritePayload::new(buffer_ready_for_write, self))
            } else {
                // Otherwise, register waker and wait.
                self.producer_waker.register(cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_write(&self, metadata: Metadata) {
        unsafe {
            *self.payload_metadata.get() = Some(metadata);
        }

        if self.registered[Operation::InPlace as usize] {
            // If a transformer is configured, transition to `Full` and wake the transformer.
            self.state.store(State::Full as u8, Ordering::Release);
            self.transformer_waker.wake();
        } else {
            self.state.store(State::Transformed as u8, Ordering::Release);
            self.consumer_waker.wake();
        }
    }
}

impl<'b, S: Storage<Item = u8> + 'b> Transformer<'b> for Slot<S> {
    async fn acquire_transform(&'b self) -> TransformPayload<'b, Self> {
        debug_assert!(self.registered[Operation::InPlace as usize], "acquire_transform called on a Slot configured without a transformer");

        poll_fn(|cx| {
            // A Transformer waits for the `Full` state.
            if self.state.compare_exchange(
                State::Full as u8, State::Transforming as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let (buffer, metadata) = unsafe {
                    let buffer_mut = self.storage.slice_mut(0..self.storage.len());
                    let buffer_ready = &mut *(buffer_mut as *mut [MaybeUninit<u8>] as *mut [u8]);
                    let metadata_ref = (*self.payload_metadata.get()).unwrap();
                    (buffer_ready, metadata_ref)
                };
                Poll::Ready(TransformPayload::new(buffer, metadata, self))
            } else {
                self.transformer_waker.register(cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_transform(&self, metadata: Metadata, remaining_length: usize) {
        trace!("Transformer releasing transform slot");
        if remaining_length > 0 {
            panic!("Transformer must consume all data in the slot. Partial consumption is not supported in Slot.");
        }

        unsafe {
            *self.payload_metadata.get() = Some(metadata);
        }
        // After transforming, the state becomes `Transformed`, ready ONLY for a consumer.
        self.state.store(State::Transformed as u8, Ordering::Release);
        // It wakes ONLY the final consumer.
        self.consumer_waker.wake();
    }
}


impl<'b, S: Storage<Item = u8> + 'b> Consumer<'b> for Slot<S> {
    async fn acquire_read(&'b self) -> ReadPayload<'b, Self> {
        debug_assert!(self.registered[Operation::Consume as usize], "acquire_read called on a Slot configured without a consumer");
        poll_fn(|cx| {
            if self.state.compare_exchange(
                State::Transformed as u8, State::Reading as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let (buffer, metadata) = unsafe {
                    // It's safe to get a mutable slice here because of the state machine guarantee.
                    let buffer_slice = self.storage.slice_mut(0..self.storage.len());
                    // But we provide an immutable view to the consumer.
                    let buffer_ref = &*(buffer_slice as *const [MaybeUninit<u8>] as *const [u8]);
                    let metadata_ref = (*self.payload_metadata.get()).unwrap();
                    (buffer_ref, metadata_ref)
                };
                Poll::Ready(ReadPayload::new(buffer, metadata, self))
            } else {
                self.consumer_waker.register(cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_read(&'b self, remaining_length: usize) {
        trace!("Consumer releasing read slot");
        if remaining_length > 0 {
            panic!("Consumer must consume all data in the slot. Partial consumption is not supported in Slot.");
        }
        // After reading, the slot is always returned to the `Empty` state.
        self.state.store(State::Empty as u8, Ordering::Release);
        // Wake the producer to start the cycle again.
        self.producer_waker.wake();
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;
    use std::sync::atomic::Ordering;

    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};

    /// Helper function to get the current state as a `State` enum.
    fn get_current_state<S: Storage<Item = u8>>(slot: &Slot<S>) -> State {
        State::try_from(slot.state.load(Ordering::SeqCst)).unwrap()
    }

    /// Test case 1: A simple pipeline without a transformer using a static buffer.
    #[tokio::test]
    async fn test_producer_consumer_flow_no_transformer() {
        // Configure the slot for a pipeline with NO transformer.
        let mut slot: StaticSlot<8> = StaticSlot::new_static();
        slot.register(Operation::Produce, PayloadSize { preferred: 4, min: 4 });
        slot.register(Operation::Consume, PayloadSize { preferred: 4, min: 4 });
        let static_slot: &'static Slot<_> = Box::leak(Box::new(slot));

        // Spawn a consumer that waits for data.
        let consumer_handle = tokio::spawn(async move {
            let payload = static_slot.acquire_read().await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
        });

        // The producer acquires the slot for writing.
        let mut write_payload = static_slot.acquire_write().await;
        assert_eq!(get_current_state(static_slot), State::Writing);
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.set_valid_length(4);
        
        drop(write_payload);
        assert_eq!(get_current_state(static_slot), State::Transformed);

        timeout(Duration::from_millis(100), consumer_handle).await.expect("Consumer timed out").unwrap();
        
        assert_eq!(get_current_state(static_slot), State::Empty);
    }

    /// Test case 2: A full pipeline using a heap-allocated buffer.
    #[tokio::test]
    async fn test_full_pipeline_flow_with_transformer_heap() {
        let mut slot = HeapSlot::new_heap(8);
        slot.register(Operation::Produce, PayloadSize { preferred: 4, min: 4 });
        slot.register(Operation::InPlace, PayloadSize { preferred: 4, min: 4 });
        slot.register(Operation::Consume, PayloadSize { preferred: 4, min: 4 });
        let static_slot: &'static Slot<_> = Box::leak(Box::new(slot));

        let (tx, rx) = oneshot::channel();

        let consumer_handle = tokio::spawn(async move {
            rx.await.expect("Failed to receive signal");
            let payload = static_slot.acquire_read().await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[11, 12, 13, 14]);
        });
        
        let transformer_handle = tokio::spawn(async move {
            let mut payload = static_slot.acquire_transform().await;
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
            for byte in payload.iter_mut() {
                *byte += 10;
            }
        });

        let mut write_payload = static_slot.acquire_write().await;
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.set_valid_length(4);
        
        drop(write_payload);
        assert_eq!(get_current_state(static_slot), State::Full);

        timeout(Duration::from_millis(100), transformer_handle).await.expect("Transformer timed out").unwrap();
        
        assert_eq!(get_current_state(static_slot), State::Transformed);

        tx.send(()).unwrap();

        timeout(Duration::from_millis(100), consumer_handle).await.expect("Consumer task timed out").unwrap();

        assert_eq!(get_current_state(static_slot), State::Empty);
    }

    /// Test case 3: Verify that the consumer waits for the transformer.
    #[tokio::test]
    async fn test_consumer_waits_for_transformer_when_configured() {
        let mut slot: StaticSlot<8> = StaticSlot::new_static();
        slot.register(Operation::InPlace, PayloadSize { preferred: 4, min: 4 });
        slot.register(Operation::Produce, PayloadSize { preferred: 4, min: 4 });
        slot.register(Operation::Consume, PayloadSize { preferred: 4, min: 4 });
        let static_slot: &'static Slot<_> = Box::leak(Box::new(slot));

        let consumer_handle = tokio::spawn(async move {
            static_slot.acquire_read().await;
        });

        tokio::task::yield_now().await;

        let mut write_payload = static_slot.acquire_write().await;
        write_payload.set_valid_length(4);
        drop(write_payload);
        
        assert_eq!(get_current_state(static_slot), State::Full);

        let result = timeout(Duration::from_millis(50), consumer_handle).await;
        assert!(result.is_err(), "Consumer did not wait for transformer and acquired the slot early");
    }
}
