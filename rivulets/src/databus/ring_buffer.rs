#![macro_use]

use core::cell::UnsafeCell;
use core::future::poll_fn;
use core::ops::Deref;
use core::task::Poll;
use core::mem::MaybeUninit;

use portable_atomic::{AtomicU32, AtomicUsize, Ordering};

use rivulets_driver::databus::{Consumer, Databus, Producer};
use rivulets_driver::payload::{Metadata, Position, ReadPayload, WritePayload};
use rivulets_driver::port::PayloadSize;

use crate::storage::Storage;
#[cfg(feature = "alloc")]
use crate::storage::Heap;
use crate::storage::Array;

use super::{ConsumerHandle, ParticipantRegistry, ProducerHandle};

pub type StaticRingBuffer<T, const N: usize> = RingBuffer<Array<T, N>>;
#[cfg(feature = "alloc")]
pub type HeapRingBuffer<T> = RingBuffer<Heap<T>>;

/// A zero-copy ring buffer implementation of Databus with side-channel metadata support.
///
/// It supports one producer and multiple consumers.
///
/// # Strict Alignment
/// To avoid complex gap handling and ensure zero-copy efficiency, this implementation
/// enforces a "Strict Alignment" policy:
/// - If `strict_alloc` is enabled, the storage length MUST be a multiple of the chunk size.
/// - The producer always writes full chunks or wraps around perfectly.
pub struct RingBuffer<S: Storage> {
    storage: S,
    boundary: PacketBoundary,
    registry: ParticipantRegistry<ProducerState, ConsumerState, ()>,
}

unsafe impl<S: Storage + Sync> Sync for RingBuffer<S> {}

/// Manages packet boundaries using atomic pointers.
/// 0 is used as the invalid/unset value.
struct PacketBoundary {
    start: AtomicU32,
    end: AtomicU32,
}

impl Default for PacketBoundary {
    fn default() -> Self {
        Self {
            start: AtomicU32::new(0),
            end: AtomicU32::new(0),
        }
    }
}

impl PacketBoundary {
    /// Updates the boundary markers based on the written payload position.
    fn update(&self, position: Position, offset: usize, len: usize) {
        match position {
            Position::Single => {
                self.start.store(offset as u32 + 1, Ordering::Release);
                self.end.store((offset + len) as u32 + 1, Ordering::Release);
            }
            Position::First => {
                self.start.store(offset as u32 + 1, Ordering::Release);
                self.end.store(0, Ordering::Release);
            }
            Position::Last => {
                self.end.store((offset + len) as u32 + 1, Ordering::Release);
            }
            Position::Middle => {}
        }
    }

    /// Determines the Metadata Position for a given read range and caps the length if an end boundary is met.
    fn calculate_position(&self, read_head: usize, available: usize) -> (Position, usize) {
        let s_raw = self.start.load(Ordering::Acquire);
        let e_raw = self.end.load(Ordering::Acquire);

        let start_off = if s_raw > 0 { Some((s_raw - 1) as usize) } else { None };
        let end_off = if e_raw > 0 { Some((e_raw - 1) as usize) } else { None };

        let is_start = start_off == Some(read_head);
        
        // If an end boundary exists within the available data, we must not read past it.
        let (is_end, actual_len) = if let Some(e) = end_off {
            if e > read_head && e <= read_head + available {
                (true, e - read_head)
            } else {
                (false, available)
            }
        } else {
            (false, available)
        };

        let pos = match (is_start, is_end) {
            (true, true) => Position::Single,
            (true, false) => Position::First,
            (false, true) => Position::Last,
            (false, false) => Position::Middle,
        };

        (pos, actual_len)
    }
}

/// State maintained for the producer.
struct ProducerState {
    write_head: AtomicUsize,
    config: UnsafeCell<ProducerConfig>,
}

impl Default for ProducerState {
    fn default() -> Self {
        Self {
            write_head: AtomicUsize::new(0),
            config: UnsafeCell::new(ProducerConfig::default()),
        }
    }
}

unsafe impl Sync for ProducerState {}

#[derive(Default, Clone, Copy)]
struct ProducerConfig {
    strict_alloc: bool,
    chunk_size: usize,
}

/// State maintained for each consumer.
#[derive(Default)]
struct ConsumerState {
    read_head: AtomicUsize,
}

impl<S: Storage> RingBuffer<S> {
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            boundary: PacketBoundary::default(),
            registry: ParticipantRegistry::new(),
        }
    }

    fn min_read_head(&self) -> usize {
        let registration = self.registry.registration();
        let count = registration.consumer_count();
        let write_head = unsafe { self.registry.producer_state().write_head.load(Ordering::Acquire) };

        if count == 0 {
            return write_head;
        }

        let mut min = usize::MAX;
        for i in 0..count {
            let cs = unsafe { self.registry.consumer_state(i) };
            let rh = cs.read_head.load(Ordering::Acquire);
            if rh < min {
                min = rh;
            }
        }

        if min == usize::MAX { write_head } else { min }
    }
}

#[cfg(feature = "alloc")]
impl<T> RingBuffer<Heap<T>> {
    pub fn new_heap(capacity: usize) -> Self {
        Self::new(Heap::new(capacity))
    }
}

impl<T, const N: usize> RingBuffer<Array<T, N>> {
    pub fn new_static() -> Self {
        Self::new(Array::from(core::array::from_fn(|_| MaybeUninit::uninit())))
    }
}
impl<S: Storage> Databus for RingBuffer<S> {
    type Item = S::Item;

    fn do_register_producer(&self, payload_size: PayloadSize, strict_alloc: bool) {
        let capacity = self.storage.len();
        let chunk_size = payload_size.preferred as usize;

        if strict_alloc {
            if capacity % chunk_size != 0 {
                panic!(
                    "RingBuffer: strict_alloc requires buffer length ({}) to be a multiple of chunk size ({}).",
                    capacity, chunk_size
                );
            }
        } else {
            if capacity % chunk_size != 0 {
                warn!(
                    "RingBuffer: buffer length ({}) is not a multiple of chunk size ({}). Wrap-around handling may reduce performance.",
                    capacity, chunk_size
                );
            }
        }

        let config = ProducerConfig {
            strict_alloc,
            chunk_size,
        };

        let state = ProducerState::default();
        unsafe { *state.config.get() = config };

        self.registry.register_producer(state);
    }

    fn do_register_consumer(&self, _payload_size: PayloadSize, _consume_all: bool) -> u8 {
        let state = ConsumerState {
            read_head: AtomicUsize::new(0),
        };
        self.registry.register_consumer(state)
    }

    fn do_register_transformer(&self, _payload_size: PayloadSize, _strict_alloc: bool, _consume_all: bool) {
        unimplemented!("RingBuffer: Transformer support is not yet implemented.");
    }
}

impl<S, P> Producer for ProducerHandle<RingBuffer<S>, P>
where
    S: Storage,
    P: Deref<Target = RingBuffer<S>> + Clone,
{
    type Item = S::Item;

    async fn acquire_write<'a>(&'a self, len: usize) -> WritePayload<'a, Self> {
        self.inner.registry.acquire_write_check();

        let producer_state = unsafe { self.inner.registry.producer_state() };
        let config = unsafe { *producer_state.config.get() };

        poll_fn(|cx| {
            let write_head = producer_state.write_head.load(Ordering::Acquire);
            let min_read_head = self.inner.min_read_head();
            
            let capacity = self.inner.storage.len();
            let free_space = capacity.saturating_sub(write_head.wrapping_sub(min_read_head));
            
            let can_write = if config.strict_alloc {
                free_space >= config.chunk_size
            } else {
                free_space > 0
            };

            if can_write {
                let physical_write_idx = write_head % capacity;
                
                let actual_len = if config.strict_alloc {
                    config.chunk_size
                } else {
                    let contiguous = capacity - physical_write_idx;
                    len.min(free_space).min(contiguous)
                };

                // println!("DEBUG: Writing actual_len={}", actual_len);

                let buffer_slice = unsafe { self.inner.storage.slice_mut(0..capacity) };
                let buffer_ptr = buffer_slice.as_mut_ptr() as *mut S::Item;
                
                let slice = unsafe {
                    core::slice::from_raw_parts_mut(buffer_ptr.add(physical_write_idx), actual_len)
                };

                Poll::Ready(WritePayload::new(slice, self))
            } else {
                self.inner.registry.register_producer_waker(cx.waker());
                Poll::Pending
            }
        })
        .await
    }

    fn release_write(&self, metadata: Metadata, written_len: usize) {
        let producer_state = unsafe { self.inner.registry.producer_state() };
        let current_write_head = producer_state.write_head.load(Ordering::Relaxed);
        
        self.inner.boundary.update(metadata.position, current_write_head, written_len);

        producer_state.write_head.fetch_add(written_len, Ordering::Release);
        self.inner.registry.wake_all_consumers();
    }
}

impl<S, P> Consumer for ConsumerHandle<RingBuffer<S>, P>
where
    S: Storage,
    P: Deref<Target = RingBuffer<S>> + Clone,
{
    type Item = S::Item;

    async fn acquire_read<'a>(&'a self, len: usize) -> ReadPayload<'a, Self> {
        self.inner.registry.acquire_read_check();
        
        let consumer_state = unsafe { self.inner.registry.consumer_state(self.id) };
        let producer_state = unsafe { self.inner.registry.producer_state() };

        poll_fn(|cx| {
            let read_head = consumer_state.read_head.load(Ordering::Acquire);
            let write_head = producer_state.write_head.load(Ordering::Acquire);
            
            let available = write_head.wrapping_sub(read_head);

            if available > 0 {
                let (position, boundary_len) = self.inner.boundary.calculate_position(read_head, available);
                
                let capacity = self.inner.storage.len();
                let physical_read_idx = read_head % capacity;
                let contiguous = capacity - physical_read_idx;

                let actual_len = len.min(boundary_len).min(contiguous);
                let metadata = Metadata::new(position);

                let buffer_slice = unsafe { self.inner.storage.slice_mut(0..capacity) };
                let buffer_ptr = buffer_slice.as_ptr() as *const S::Item;

                let slice = unsafe {
                    core::slice::from_raw_parts(buffer_ptr.add(physical_read_idx), actual_len)
                };

                Poll::Ready(ReadPayload::new(slice, metadata, self))
            } else {
                self.inner.registry.register_consumer_waker(self.id, cx.waker());
                Poll::Pending
            }
        })
        .await
    }

    fn release_read(&self, _metadata: Metadata, consumed_len: usize) {
        let consumer_state = unsafe { self.inner.registry.consumer_state(self.id) };
        
        consumer_state.read_head.fetch_add(consumed_len, Ordering::Release);
        self.inner.registry.wake_producer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Array;
    use std::sync::Arc;
    use tokio::time::{timeout, Duration};

    // Alias for easier test setup using static storage
    type TestRing<const N: usize> = RingBuffer<Array<u8, N>>;

    #[tokio::test]
    async fn test_basic_produce_consume_strict() {
        // Buffer size 16, chunk size 4. Strictly aligned.
        let ring = Arc::new(TestRing::<16>::new_static());
        
        let producer = ProducerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);
        let consumer = ConsumerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);

        // Write strict chunk
        let mut w_payload = producer.acquire_write(4).await;
        w_payload[..].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        w_payload.set_position(Position::Single);
        w_payload.commit_all();
        drop(w_payload);

        // Read back
        let mut r_payload = consumer.acquire_read(4).await;
        assert_eq!(&r_payload[..], &[0xAA, 0xBB, 0xCC, 0xDD]);
        
        // Metadata position defaults to Single in commit_all if not specified otherwise
        assert_eq!(r_payload.position(), Position::Single);
        r_payload.commit_all();
    }

    #[tokio::test]
    async fn test_producer_wrap_around() {
        // Capacity 8. Write 4 + 4 + 4. The third write wraps physically to index 0.
        let ring = Arc::new(TestRing::<8>::new_static());
        
        let producer = ProducerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);
        let consumer = ConsumerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);

        // Fill buffer
        for i in 0..2 {
            let mut w = producer.acquire_write(4).await;
            w.fill(i as u8);
            w.commit_all();
        }

        // Drain first 4 bytes
        let mut r = consumer.acquire_read(4).await;
        assert_eq!(r[0], 0);
        r.commit_all();
        drop(r);

        // Write causing wrap-around
        let mut w = producer.acquire_write(4).await;
        w.fill(2); // Should physically be at index 0..4
        w.commit_all();
        drop(w);

        // Read remaining
        let mut r2 = consumer.acquire_read(4).await;
        assert_eq!(r2[0], 1);
        r2.commit_all();
        drop(r2);

        let mut r3 = consumer.acquire_read(4).await;
        assert_eq!(r3[0], 2);
        r3.commit_all();
    }

    #[tokio::test]
    async fn test_backpressure_full_buffer() {
        let ring = Arc::new(TestRing::<4>::new_static());
        
        let producer = ProducerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);
        let consumer = ConsumerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);

        // Fill buffer
        let mut w = producer.acquire_write(4).await;
        w.commit_all();
        drop(w);

        // Second write should block until consumer reads
        let producer_task = tokio::spawn(async move {
            let mut w = producer.acquire_write(4).await;
            w[0] = 0xFF;
            w.commit_all();
        });

        // Ensure producer is actually blocked
        tokio::task::yield_now().await;
        assert!(!producer_task.is_finished());

        // Consume to free space
        let mut r = consumer.acquire_read(4).await;
        r.commit_all();
        drop(r);

        // Producer should complete now
        timeout(Duration::from_millis(100), producer_task)
            .await
            .expect("Producer did not wake up after consume")
            .unwrap();

        // Verify new data
        let r_new = consumer.acquire_read(4).await;
        assert_eq!(r_new[0], 0xFF);
    }

    #[tokio::test]
    async fn test_multiple_consumers_slowest_reader() {
        let ring = Arc::new(TestRing::<8>::new_static());
        
        let producer = ProducerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);
        let fast_consumer = ConsumerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);
        let slow_consumer = ConsumerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);

        // Write 1: Occupy 0..4
        let mut w1 = producer.acquire_write(4).await;
        w1.commit_all();
        drop(w1);

        // Write 2: Occupy 4..8 (Buffer Full)
        let mut w2 = producer.acquire_write(4).await;
        w2.commit_all();
        drop(w2);

        // Fast consumer reads everything
        fast_consumer.acquire_read(4).await.commit_all();
        fast_consumer.acquire_read(4).await.commit_all();

        // Producer should still be blocked because slow_consumer hasn't read 0..4 yet
        let try_write = tokio::spawn(async move {
             producer.acquire_write(4).await.commit_all();
        });

        tokio::task::yield_now().await;
        assert!(!try_write.is_finished(), "Producer overwrote unread data from slow consumer");

        // Slow consumer advances
        slow_consumer.acquire_read(4).await.commit_all();

        // Now producer can proceed (overwriting 0..4)
        timeout(Duration::from_millis(100), try_write)
            .await
            .expect("Producer failed to acquire after slow consumer read")
            .unwrap();
    }

    #[tokio::test]
    async fn test_packet_assembly_and_boundary_truncation() {
        let ring = Arc::new(TestRing::<16>::new_static());
        let producer = ProducerHandle::new(ring.clone(), PayloadSize::new(4, 4), false);
        let consumer = ConsumerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);

        // Test valid packet merging (First + Last -> Single)
        
        // Write "Start"
        let mut w1 = producer.acquire_write(4).await;
        w1.set_position(Position::First);
        w1.commit(4);
        drop(w1);

        // Write "End"
        let mut w2 = producer.acquire_write(4).await;
        w2.set_position(Position::Last);
        w2.commit(4);
        drop(w2);

        // Verify: Consumer sees one contiguous "Single" packet
        let mut r = consumer.acquire_read(8).await;
        assert_eq!(r.len(), 8);
        assert_eq!(r.position(), Position::Single);
        r.commit_all();
        drop(r);

        // Test Read Truncation
        // Verify that reading assumes packet boundaries even if requested length is larger.

        // Write a standard Single packet (4 bytes)
        let mut w3 = producer.acquire_write(4).await;
        w3.set_position(Position::Single);
        w3.commit(4);
        drop(w3);

        // Action: Request 10 bytes
        // Expectation: RingBuffer limits the read to the packet size (4 bytes)
        let mut r2 = consumer.acquire_read(10).await;
        
        assert_eq!(r2.len(), 4, "Read should be truncated at the packet boundary");
        assert_eq!(r2.position(), Position::Single);
        r2.commit_all();
    }

    #[tokio::test]
    #[should_panic(expected = "strict_alloc requires buffer length")]
    async fn test_strict_alloc_panic_on_misalignment() {
        // Buffer 10 is not multiple of chunk 4
        let ring = Arc::new(TestRing::<10>::new_static());
        ProducerHandle::new(ring, PayloadSize::new(4, 4), true);
    }

    #[tokio::test]
    async fn test_non_strict_variable_length_writes() {
        let ring = Arc::new(TestRing::<16>::new_static());
        let producer = ProducerHandle::new(ring.clone(), PayloadSize::new(4, 4), false);
        let consumer = ConsumerHandle::new(ring.clone(), PayloadSize::new(4, 4), true);

        // Write 3 bytes
        let mut w = producer.acquire_write(3).await;
        assert_eq!(w.len(), 3);
        w.commit(3);
        drop(w);

        let mut r = consumer.acquire_read(10).await;
        assert_eq!(r.len(), 3);
        r.commit_all();
    }
}