#![macro_use]

use core::cell::UnsafeCell;
use core::future::poll_fn;
use core::ops::Deref;
use core::task::Poll;

use portable_atomic::{AtomicU32, AtomicUsize, Ordering};

use rivulets_driver::databus::{Consumer, Databus, Producer, Transformer};
use rivulets_driver::payload::{Metadata, Position, ReadPayload, TransformPayload, WritePayload};
use rivulets_driver::port::PayloadSize;

use crate::storage::Storage;

use super::{ConsumerHandle, ParticipantRegistry, ProducerHandle, TransformerHandle};

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

impl<S, P> Transformer for TransformerHandle<RingBuffer<S>, P>
where
    S: Storage,
    P: Deref<Target = RingBuffer<S>> + Clone,
{
    type Item = S::Item;

    async fn acquire_transform<'a>(&'a self, _len: usize) -> TransformPayload<'a, Self> {
        unimplemented!("RingBuffer: Transformer not implemented")
    }

    fn release_transform(&self, _metadata: Metadata, _transformed_len: usize) {
        unimplemented!("RingBuffer: Transformer not implemented")
    }
}