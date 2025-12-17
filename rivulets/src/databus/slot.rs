#![macro_use]

use core::cell::UnsafeCell;
use core::future::poll_fn;
use core::mem::MaybeUninit;
use core::ops::Deref;
use core::task::Poll;

use portable_atomic::{AtomicU8, Ordering};

use rivulets_driver::databus::{Consumer, Databus, Producer, Transformer};
use rivulets_driver::payload::{Metadata, ReadPayload, TransformPayload, WritePayload};
use rivulets_driver::port::PayloadSize;

use crate::storage::Storage;
#[cfg(feature = "alloc")]
use crate::storage::Heap;
use crate::storage::Array;

#[cfg(feature = "alloc")]
extern crate alloc;

use super::{ConsumerHandle, ProducerHandle, TransformerHandle, ParticipantRegistry};

pub type StaticSlot<T, const N: usize> = Slot<Array<T, N>>;
#[cfg(feature = "alloc")]
pub type HeapSlot<T> = Slot<Heap<T>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum State {
    Empty,
    Writing,
    Full,
    Transforming,
    ReadyForRead,
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
            x if x == State::Writing as u8 => Ok(State::Writing),
            x if x == State::Full as u8 => Ok(State::Full),
            x if x == State::Transforming as u8 => Ok(State::Transforming),
            x if x == State::ReadyForRead as u8 => Ok(State::ReadyForRead),
            _ => Err(()),
        }
    }
}

pub struct Slot<S: Storage> {
    storage: S,
    // Stores the metadata and the valid length of data in the buffer
    payload_info: UnsafeCell<Option<(Metadata, usize)>>,
    state: AtomicU8,
    registry: ParticipantRegistry<bool, (), ()>,
}

unsafe impl<S: Storage + Sync> Sync for Slot<S> {}

impl<S: Storage> Slot<S> {
    pub fn new(storage: S) -> Self {
        Slot {
            storage,
            payload_info: UnsafeCell::new(None),
            state: AtomicU8::new(State::Empty as u8),
            registry: ParticipantRegistry::new(),
        }
    }

    pub fn get_current_metadata(&self) -> Option<Metadata> {
        unsafe { (*self.payload_info.get()).map(|(m, _)| m) }
    }
}

impl<S: Storage> Databus for Slot<S> {
    type Item = S::Item;

    fn do_register_producer(&self, payload_size: PayloadSize, strict_alloc: bool) {
        self.registry.register_producer(strict_alloc);
        // Slot natively supports exact writes up to its capacity, so we don't strictly need to store the exact flag yet.
        if payload_size.preferred as usize > self.storage.len() {
            warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
    }

    fn do_register_consumer(&self, payload_size: PayloadSize, consume_all: bool) -> u8 {
        assert!(consume_all, "Slot consumer must consume all data in the slot.");
        let id = self.registry.register_consumer(());
        if payload_size.preferred as usize > self.storage.len() {
            warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
        id
    }

    fn do_register_transformer(&self, payload_size: PayloadSize, _strict_alloc: bool, consume_all: bool) {
        assert!(consume_all, "Slot transformer must consume all data in the slot.");
        self.registry.register_transformer(());
        if payload_size.preferred as usize > self.storage.len() {
             warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
    }
}

#[cfg(feature = "alloc")]
impl<T> Slot<Heap<T>> {
    pub fn new_heap(capacity: usize) -> Self {
        Self::new(Heap::new(capacity))
    }
}

impl<T, const N: usize> Slot<Array<T, N>> {
    pub fn new_static() -> Self {
        Self::new(Array::from(core::array::from_fn(|_| MaybeUninit::uninit())))
    }
}

impl<S, P> Producer for ProducerHandle<Slot<S>, P> 
where 
    S: Storage,
    P: Deref<Target=Slot<S>> + Clone,
{
    type Item = S::Item;

    async fn acquire_write<'a>(&'a self, len: usize) -> WritePayload<'a, Self> {
        self.inner.registry.acquire_write_check();
        if unsafe { *self.inner.registry.producer_state() } {
            assert!(len < self.inner.storage.len());
        }
        let payload_len = self.inner.storage.len().min(len);

        poll_fn(|cx| {
            if self.inner.state.compare_exchange(
                State::Empty as u8, State::Writing as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let buffer = unsafe { self.inner.storage.slice_mut(0..self.inner.storage.len()) };
                let buffer_ready = unsafe { &mut *(buffer as *mut [MaybeUninit<S::Item>] as *mut [S::Item]) };
                // Pass the slice limited by requested length
                Poll::Ready(WritePayload::new(&mut buffer_ready[0..payload_len], self))
            } else {
                self.inner.registry.register_producer_waker(cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_write(&self, metadata: Metadata, written_len: usize) {
        unsafe {
            *self.inner.payload_info.get() = Some((metadata, written_len));
        }
        let registration = self.inner.registry.registration();

        if registration.transformer() {
            self.inner.state.store(State::Full as u8, Ordering::Release);
            self.inner.registry.wake_transformer();
        } else {
            self.inner.registry.reset_consumers_finished();
            self.inner.state.store(State::ReadyForRead as u8, Ordering::Release);
            self.inner.registry.wake_all_consumers();
        }
    }
}

impl<S, P> Transformer for TransformerHandle<Slot<S>, P> 
where 
    S: Storage,
    P: Deref<Target=Slot<S>> + Clone,
{
    type Item = S::Item;

    async fn acquire_transform<'a>(&'a self, len: usize) -> TransformPayload<'a, Self> {
        self.inner.registry.acquire_transform_check();
        
        poll_fn(|cx| {
            if self.inner.state.compare_exchange(
                State::Full as u8, State::Transforming as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let (_, valid_len) = unsafe { (*self.inner.payload_info.get()).unwrap() };
                assert!(valid_len <= len);

                let (buffer, metadata) = unsafe {
                    let buffer_mut = self.inner.storage.slice_mut(0..self.inner.storage.len());
                    let buffer_ready = &mut *(buffer_mut as *mut [MaybeUninit<S::Item>] as *mut [S::Item]);
                    let (metadata_ref, _) = (*self.inner.payload_info.get()).unwrap();
                    (&mut buffer_ready[0..valid_len], metadata_ref)
                };
                Poll::Ready(TransformPayload::new(buffer, metadata, self))
            } else {
                self.inner.registry.register_transformer_waker(cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_transform(&self, metadata: Metadata, transformed_len: usize) {
        trace!("Transformer releasing transform slot");
        let (_, input_len) = unsafe { (*self.inner.payload_info.get()).unwrap() };
        
        if transformed_len > input_len {
             panic!("Transformer expanded data beyond input length, which is unsafe in this Slot implementation.");
        }

        unsafe {
            *self.inner.payload_info.get() = Some((metadata, transformed_len));
        }
        self.inner.registry.reset_consumers_finished();
        self.inner.state.store(State::ReadyForRead as u8, Ordering::Release);
        self.inner.registry.wake_all_consumers();
    }
}

impl<S, P> Consumer for ConsumerHandle<Slot<S>, P> 
where 
    S: Storage,
    P: Deref<Target=Slot<S>> + Clone,
{
    type Item = S::Item;

    async fn acquire_read<'a>(&'a self, len: usize) -> ReadPayload<'a, Self> {
        self.inner.registry.acquire_read_check();        

        poll_fn(|cx| {
            if self.inner.registry.is_consumer_finished(self.id) {
                self.inner.registry.register_consumer_waker(self.id, cx.waker());
                return Poll::Pending;
            }

            if self.inner.state.load(Ordering::Acquire) == State::ReadyForRead as u8 {
                let (_, valid_len) = unsafe { (*self.inner.payload_info.get()).unwrap() };
                assert!(valid_len <= len);

                let (buffer, metadata) = unsafe {
                    let buffer_slice = self.inner.storage.slice_mut(0..self.inner.storage.len());
                    let buffer_ref = &*(buffer_slice as *const [MaybeUninit<S::Item>] as *const [S::Item]);
                    let (metadata_ref, _) = (*self.inner.payload_info.get()).unwrap();
                    (&buffer_ref[0..valid_len], metadata_ref)
                };
                Poll::Ready(ReadPayload::new(buffer, metadata, self))
            } else {
                self.inner.registry.register_consumer_waker(self.id, cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_read(&self, _metadata: Metadata, consumed_len: usize) {
        trace!("Consumer {} releasing read slot", self.id);
        
        let (_, total_len) = unsafe { (*self.inner.payload_info.get()).unwrap() };

        if consumed_len < total_len {
            panic!("Consumer must consume all data in the slot. Partial consumption is not supported in Slot.");
        }

        if self.inner.registry.mark_consumer_as_finished(self.id) {
            trace!("Last consumer finished. Resetting slot.");
            self.inner.state.store(State::Empty as u8, Ordering::Release);
            self.inner.registry.wake_producer();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};

    fn get_current_state<S: Storage<Item = u8>>(slot: &Slot<S>) -> State {
        State::try_from(slot.state.load(Ordering::SeqCst)).unwrap()
    }

    #[tokio::test]
    async fn test_producer_consumer_flow_no_transformer() {
        let slot: StaticSlot<u8, 8> = StaticSlot::new_static();
        let static_slot: &'static Slot<_> = Box::leak(Box::new(slot));
        
        let producer = ProducerHandle::new(static_slot, PayloadSize::new(4, 4), false);
        let consumer = ConsumerHandle::new(static_slot, PayloadSize::new(4, 4), true);

        let consumer_handle = tokio::spawn(async move {
            let mut payload = consumer.acquire_read(8).await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
            payload.commit_all();
        });

        let mut write_payload = producer.acquire_write(8).await;
        assert_eq!(get_current_state(static_slot), State::Writing);
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.commit(4);
        
        drop(write_payload);
        assert_eq!(get_current_state(static_slot), State::ReadyForRead);

        timeout(Duration::from_millis(100), consumer_handle).await.expect("Consumer timed out").unwrap();
        
        assert_eq!(get_current_state(static_slot), State::Empty);
    }

    #[tokio::test]
    #[cfg(feature="alloc")]
    async fn test_full_pipeline_flow_with_transformer_heap() {
        let slot = HeapSlot::<u8>::new_heap(8);
        let slot = Arc::new(slot);

        let producer = ProducerHandle::new(slot.clone(), PayloadSize::new(4, 4), false);
        let transformer = TransformerHandle::new(slot.clone(), PayloadSize::new(4, 4), false, true);
        let consumer = ConsumerHandle::new(slot.clone(), PayloadSize::new(4, 4), true);

        let (tx, rx) = oneshot::channel();

        let consumer_handle = tokio::spawn(async move {
            rx.await.expect("Failed to receive signal");
            let mut payload = consumer.acquire_read(8).await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[11, 12, 13, 14]);
            payload.commit_all();
        });
        
        let transformer_handle = tokio::spawn(async move {
            let mut payload = transformer.acquire_transform(8).await;
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
            for byte in payload.iter_mut() {
                *byte += 10;
            }
            payload.commit_all();
        });

        let mut write_payload = producer.acquire_write(8).await;
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.commit(4);
        
        drop(write_payload);
        assert_eq!(get_current_state(&slot), State::Full);

        timeout(Duration::from_millis(100), transformer_handle).await.expect("Transformer timed out").unwrap();
        
        assert_eq!(get_current_state(&slot), State::ReadyForRead);

        tx.send(()).unwrap();

        timeout(Duration::from_millis(100), consumer_handle).await.expect("Consumer task timed out").unwrap();

        assert_eq!(get_current_state(&slot), State::Empty);
    }
    
    #[tokio::test]
    async fn test_consumer_waits_for_transformer_when_configured() {
        let slot: StaticSlot<u8, 8> = StaticSlot::new_static();
        let slot = Arc::new(slot);

        let producer = ProducerHandle::new(slot.clone(), PayloadSize::new(4, 4), false);
        TransformerHandle::new(slot.clone(), PayloadSize::new(4, 4), false, true);
        let consumer = ConsumerHandle::new(slot.clone(), PayloadSize::new(4, 4), true);

        let consumer_handle = tokio::spawn(async move {
            let mut payload = consumer.acquire_read(4).await;
            payload.commit_all();
        });

        tokio::task::yield_now().await;

        let mut write_payload = producer.acquire_write(4).await;
        write_payload.commit_all();
        drop(write_payload);
        
        assert_eq!(get_current_state(&slot), State::Full);

        let result = timeout(Duration::from_millis(50), consumer_handle).await;
        assert!(result.is_err(), "Consumer did not wait for transformer and acquired the slot early");
    }

    #[tokio::test]
    async fn test_two_consumers_flow() {
        let slot: StaticSlot<u8, 8> = StaticSlot::new_static();
        let slot = Arc::new(slot);

        let producer = ProducerHandle::new(slot.clone(), PayloadSize::new(4, 4), false);
        let consumer1 = ConsumerHandle::new(slot.clone(), PayloadSize::new(4, 4), true);
        let consumer2 = ConsumerHandle::new(slot.clone(), PayloadSize::new(4, 4), true);
        
        let producer_handle = tokio::spawn({
            async move {
                let mut write_payload = producer.acquire_write(4).await;
                write_payload[0..4].copy_from_slice(&[10, 20, 30, 40]);
                write_payload.commit(4);
            }
        });

        producer_handle.await.unwrap();
        assert_eq!(get_current_state(&slot), State::ReadyForRead);

        let (c1_tx, c1_rx) = oneshot::channel();

        let consumer1_handle = tokio::spawn(async move {
            let mut payload = consumer1.acquire_read(4).await;
            assert_eq!(&payload[..], &[10, 20, 30, 40]);
            payload.commit_all();
        });

        let consumer2_handle = tokio::spawn(async move {
            c1_rx.await.unwrap();
            let mut payload = consumer2.acquire_read(4).await;
            assert_eq!(&payload[..], &[10, 20, 30, 40]);
            payload.commit_all();
        });

        consumer1_handle.await.unwrap();

        // One consumer finished, but one remains, so state should still be ReadyForRead
        assert_eq!(get_current_state(&slot), State::ReadyForRead);
        c1_tx.send(()).unwrap();

        consumer2_handle.await.unwrap();

        // Both finished, now it should be Empty
        assert_eq!(get_current_state(&slot), State::Empty);
    }
}
