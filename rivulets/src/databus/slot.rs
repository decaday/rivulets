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

pub type StaticSlot<const N: usize> = Slot<Array<u8, N>>;
#[cfg(feature = "alloc")]
pub type HeapSlot = Slot<Heap<u8>>;

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

pub struct Slot<S: Storage<Item = u8>> {
    storage: S,
    payload_metadata: UnsafeCell<Option<Metadata>>,
    // vaild_length: AtomicUsize,
    state: AtomicU8,
    registry: ParticipantRegistry<(), (), ()>,
}

unsafe impl<S: Storage<Item = u8> + Sync> Sync for Slot<S> {}

impl<S: Storage<Item = u8>> Slot<S> {
    pub fn new(storage: S) -> Self {
        Slot {
            storage,
            payload_metadata: UnsafeCell::new(None),
            state: AtomicU8::new(State::Empty as u8),
            registry: ParticipantRegistry::new(),
        }
    }

    pub fn get_current_metadata(&self) -> Option<Metadata> {
        unsafe { (*self.payload_metadata.get()).clone() }
    }
}

impl<S: Storage<Item=u8>> Databus for Slot<S> {
    fn do_register_producer(&self, payload_size: PayloadSize) {
        self.registry.register_producer(());
        if payload_size.preferred as usize > self.storage.len() {
            warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
    }

    fn do_register_consumer(&self, payload_size: PayloadSize) -> u8 {
        let id = self.registry.register_consumer(());
        if payload_size.preferred as usize > self.storage.len() {
            warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
        id
    }

    fn do_register_transformer(&self, payload_size: PayloadSize) {
        self.registry.register_transformer(());
        if payload_size.preferred as usize > self.storage.len() {
             warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
    }
}

#[cfg(feature = "alloc")]
impl Slot<Heap<u8>> {
    pub fn new_heap(capacity: usize) -> Self {
        Self::new(Heap::new(capacity))
    }
}

impl<const N: usize> Slot<Array<u8, N>> {
    pub fn new_static() -> Self {
        Self::new(Array::from([MaybeUninit::uninit(); N]))
    }
}

impl<S, P> Producer for ProducerHandle<Slot<S>, P> 
where 
    S: Storage<Item = u8>,
    P: Deref<Target=Slot<S>> + Clone,
{
    async fn acquire_write<'a>(&'a self, len: usize, exact: bool) -> WritePayload<'a, Self> {
        self.inner.registry.acquire_write_check();
        if exact {
            assert!(len < self.inner.storage.len());
        }
        let payload_len = self.inner.storage.len().min(len);

        poll_fn(|cx| {
            if self.inner.state.compare_exchange(
                State::Empty as u8, State::Writing as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let buffer = unsafe { self.inner.storage.slice_mut(0..self.inner.storage.len()) };
                let buffer_ready_for_write = unsafe { &mut *(buffer as *mut [MaybeUninit<u8>] as *mut [u8]) };
                Poll::Ready(WritePayload::new(&mut buffer_ready_for_write[0..payload_len], self))
            } else {
                self.inner.registry.register_producer_waker(cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_write(&self, metadata: Metadata) {
        unsafe {
            *self.inner.payload_metadata.get() = Some(metadata);
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
    S: Storage<Item = u8>,
    P: Deref<Target=Slot<S>> + Clone,
{
    async fn acquire_transform<'a>(&'a self, len: usize) -> TransformPayload<'a, Self> {
        self.inner.registry.acquire_transform_check();
        let payload_len = unsafe { (*self.inner.payload_metadata.get()).unwrap().valid_length };
        assert!(payload_len <= len);
        
        poll_fn(|cx| {
            if self.inner.state.compare_exchange(
                State::Full as u8, State::Transforming as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let (buffer, metadata) = unsafe {
                    let buffer_mut = self.inner.storage.slice_mut(0..self.inner.storage.len());
                    let buffer_ready = &mut *(buffer_mut as *mut [MaybeUninit<u8>] as *mut [u8]);
                    let metadata_ref = (*self.inner.payload_metadata.get()).unwrap();
                    (&mut buffer_ready[0..payload_len], metadata_ref)
                };
                Poll::Ready(TransformPayload::new(buffer, metadata, self))
            } else {
                self.inner.registry.register_transformer_waker(cx.waker());
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
            *self.inner.payload_metadata.get() = Some(metadata);
        }
        self.inner.registry.reset_consumers_finished();
        self.inner.state.store(State::ReadyForRead as u8, Ordering::Release);
        self.inner.registry.wake_all_consumers();
    }
}

impl<S, P> Consumer for ConsumerHandle<Slot<S>, P> 
where 
    S: Storage<Item = u8>,
    P: Deref<Target=Slot<S>> + Clone,
{
    async fn acquire_read<'a>(&'a self, len: usize) -> ReadPayload<'a, Self> {
        self.inner.registry.acquire_read_check();

            let payload_len = unsafe { (*self.inner.payload_metadata.get()).unwrap().valid_length };
        assert!(payload_len <= len);

        poll_fn(|cx| {
            if self.inner.registry.is_consumer_finished(self.id) {
                self.inner.registry.register_consumer_waker(self.id, cx.waker());
                return Poll::Pending;
            }

            if self.inner.state.load(Ordering::Acquire) == State::ReadyForRead as u8 {
                let (buffer, metadata) = unsafe {
                    let buffer_slice = self.inner.storage.slice_mut(0..self.inner.storage.len());
                    let buffer_ref = &*(buffer_slice as *const [MaybeUninit<u8>] as *const [u8]);
                    let metadata_ref = (*self.inner.payload_metadata.get()).unwrap();
                    (&buffer_ref[0..payload_len], metadata_ref)
                };
                Poll::Ready(ReadPayload::new(buffer, metadata, self))
            } else {
                self.inner.registry.register_consumer_waker(self.id, cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_read(&self, remaining_length: usize) {
        trace!("Consumer {} releasing read slot", self.id);
        if remaining_length > 0 {
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
        let slot: StaticSlot<8> = StaticSlot::new_static();
        let static_slot: &'static Slot<_> = Box::leak(Box::new(slot));
        
        let producer = ProducerHandle::new(static_slot, PayloadSize { preferred: 4, min: 4 });
        let consumer = ConsumerHandle::new(static_slot, PayloadSize { preferred: 4, min: 4 });

        let consumer_handle = tokio::spawn(async move {
            let payload = consumer.acquire_read(8).await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
            consumer.release_read(0);
        });

        let mut write_payload = producer.acquire_write(8, false).await;
        assert_eq!(get_current_state(static_slot), State::Writing);
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.set_valid_length(4);
        
        drop(write_payload);
        assert_eq!(get_current_state(static_slot), State::ReadyForRead);

        timeout(Duration::from_millis(100), consumer_handle).await.expect("Consumer timed out").unwrap();
        
        assert_eq!(get_current_state(static_slot), State::Empty);
    }

    #[tokio::test]
    #[cfg(feature="alloc")]
    async fn test_full_pipeline_flow_with_transformer_heap() {
        let slot = HeapSlot::new_heap(8);
        let slot = Arc::new(slot);

        let producer = ProducerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        let transformer = TransformerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        let consumer = ConsumerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });

        let (tx, rx) = oneshot::channel();

        let consumer_handle = tokio::spawn(async move {
            rx.await.expect("Failed to receive signal");
            let payload = consumer.acquire_read(8).await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[11, 12, 13, 14]);
            consumer.release_read(0);
        });
        
        let transformer_handle = tokio::spawn(async move {
            let mut payload = transformer.acquire_transform(8).await;
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
            for byte in payload.iter_mut() {
                *byte += 10;
            }
            transformer.release_transform(payload.metadata, 0);
        });

        let mut write_payload = producer.acquire_write(8, false).await;
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.set_valid_length(4);
        
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
        let slot: StaticSlot<8> = StaticSlot::new_static();
        let slot = Arc::new(slot);

        let producer = ProducerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        TransformerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        let consumer = ConsumerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });

        let consumer_handle = tokio::spawn(async move {
            consumer.acquire_read(4).await;
        });

        tokio::task::yield_now().await;

        let mut write_payload = producer.acquire_write(4, false).await;
        write_payload.set_valid_length(4);
        drop(write_payload);
        
        assert_eq!(get_current_state(&slot), State::Full);

        let result = timeout(Duration::from_millis(50), consumer_handle).await;
        assert!(result.is_err(), "Consumer did not wait for transformer and acquired the slot early");
    }

    #[tokio::test]
    async fn test_two_consumers_flow() {
        let slot: StaticSlot<8> = StaticSlot::new_static();
        let slot = Arc::new(slot);

        let producer = ProducerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        let consumer1 = ConsumerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        let consumer2 = ConsumerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        
        let producer_handle = tokio::spawn({
            async move {
                let mut write_payload = producer.acquire_write(4, false).await;
                write_payload[0..4].copy_from_slice(&[10, 20, 30, 40]);
                write_payload.set_valid_length(4);
            }
        });

        producer_handle.await.unwrap();
        assert_eq!(get_current_state(&slot), State::ReadyForRead);

        let (c1_tx, c1_rx) = oneshot::channel();

        let consumer1_handle = tokio::spawn(async move {
            let payload = consumer1.acquire_read(4).await;
            assert_eq!(&payload[..], &[10, 20, 30, 40]);
            consumer1.release_read(0);
        });

        let consumer2_handle = tokio::spawn(async move {
            c1_rx.await.unwrap();
            let payload = consumer2.acquire_read(4).await;
            assert_eq!(&payload[..], &[10, 20, 30, 40]);
            consumer2.release_read(0);
        });

        consumer1_handle.await.unwrap();

        assert_eq!(get_current_state(&slot), State::ReadyForRead);
        c1_tx.send(()).unwrap();

        consumer2_handle.await.unwrap();

        assert_eq!(get_current_state(&slot), State::Empty);
    }
}
