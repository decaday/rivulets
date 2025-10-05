#![macro_use]

use core::cell::UnsafeCell;
use core::future::poll_fn;
use core::mem::MaybeUninit;
use core::task::Poll;

use bitfield_struct::bitfield;
use embassy_sync::waitqueue::AtomicWaker;
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
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "alloc")]
use crate::Mutex;

use super::{ProducerHandle, ConsumerHandle, TransformerHandle};

#[cfg(not(feature = "alloc"))]
const MAX_CONSUMERS: usize = 2;
#[cfg(feature = "alloc")]
const MAX_CONSUMERS: usize = 8;

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
    Transformed,
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

#[bitfield(u8)]
#[derive(PartialEq, Eq)]
struct Registration {
    producer: bool,
    transformer: bool,
    #[bits(6)]
    consumer_count: u8,
}

pub struct Slot<S: Storage<Item = u8>> {
    storage: S,
    payload_metadata: UnsafeCell<Option<Metadata>>,
    state: AtomicU8,
    producer_waker: AtomicWaker,
    transformer_waker: AtomicWaker,
    
    #[cfg(not(feature = "alloc"))]
    consumer_wakers: [AtomicWaker; MAX_CONSUMERS],
    #[cfg(feature = "alloc")]
    consumer_wakers: Mutex<Vec<AtomicWaker>>,

    registered: AtomicU8,
    consumers_finished: AtomicU8,
}

unsafe impl<S: Storage<Item = u8> + Sync> Sync for Slot<S> {}

impl<S: Storage<Item = u8>> Slot<S> {
    pub fn new(storage: S) -> Self {
        Slot {
            storage,
            payload_metadata: UnsafeCell::new(None),
            state: AtomicU8::new(State::Empty as u8),
            producer_waker: AtomicWaker::new(),
            transformer_waker: AtomicWaker::new(),
            #[cfg(not(feature = "alloc"))]
            consumer_wakers: core::array::from_fn(|_| AtomicWaker::new()),
            #[cfg(feature = "alloc")]
            consumer_wakers: Mutex::new(Vec::new()),
            registered: AtomicU8::new(Registration::new().into()),
            consumers_finished: AtomicU8::new(0),
        }
    }

    pub fn get_current_metadata(&self) -> Option<Metadata> {
        unsafe { (*self.payload_metadata.get()).clone() }
    }
}

impl<S: Storage<Item=u8>> Databus for Slot<S> {
    fn do_register_producer(&self, payload_size: PayloadSize) {
        loop {
            let current_val = self.registered.load(Ordering::Relaxed);
            let current_reg: Registration = current_val.into();
            if current_reg.producer() {
                panic!("Producer already registered");
            }
            let new_reg = current_reg.with_producer(true);
            if self.registered.compare_exchange_weak(current_val, new_reg.into(), Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                break;
            }
        }

        if payload_size.preferred as usize > self.storage.len() {
            warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
    }

    fn do_register_consumer(&self, payload_size: PayloadSize) -> u8 {
        let id = loop {
            let current_val = self.registered.load(Ordering::Relaxed);
            let current_reg: Registration = current_val.into();
            let count = current_reg.consumer_count();

            if count as usize >= MAX_CONSUMERS {
                panic!("Maximum number of consumers ({}) reached", MAX_CONSUMERS);
            }

            let new_reg = current_reg.with_consumer_count(count + 1);
            if self.registered.compare_exchange_weak(current_val, new_reg.into(), Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                break count;
            }
        };

        #[cfg(feature = "alloc")]
        {
            let mut wakers = self.consumer_wakers.lock().unwrap();
            wakers.push(AtomicWaker::new());
        }

        if payload_size.preferred as usize > self.storage.len() {
            warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
        }
        id
    }

    fn do_register_transformer(&self, payload_size: PayloadSize) {
        loop {
            let current_val = self.registered.load(Ordering::Relaxed);
            let current_reg: Registration = current_val.into();
            if current_reg.transformer() {
                panic!("Transformer already registered");
            }
            let new_reg = current_reg.with_transformer(true);
            if self.registered.compare_exchange_weak(current_val, new_reg.into(), Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                break;
            }
        }
        
        if payload_size.preferred as usize > self.storage.len() {
             // warn!("Slot buffer({}) is smaller than preferred size({})", self.storage.len(), payload_size.preferred);
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

impl<'a, S: Storage<Item = u8> + 'a> Producer<'a> for ProducerHandle<Slot<S>> {
    async fn acquire_write(&'a self) -> WritePayload<'a, Self> {
        let registration: Registration = self.inner.registered.load(Ordering::Relaxed).into();
        assert!(registration.producer(), "acquire_write called on a Slot configured without a producer");
        poll_fn(|cx| {
            if self.inner.state.compare_exchange(
                State::Empty as u8, State::Writing as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let buffer = unsafe { self.inner.storage.slice_mut(0..self.inner.storage.len()) };
                let buffer_ready_for_write = unsafe { &mut *(buffer as *mut [MaybeUninit<u8>] as *mut [u8]) };
                Poll::Ready(WritePayload::new(buffer_ready_for_write, self))
            } else {
                self.inner.producer_waker.register(cx.waker());
                Poll::Pending
            }
        }).await
    }

    fn release_write(&self, metadata: Metadata) {
        unsafe {
            *self.inner.payload_metadata.get() = Some(metadata);
        }
        let registration: Registration = self.inner.registered.load(Ordering::Relaxed).into();

        if registration.transformer() {
            self.inner.state.store(State::Full as u8, Ordering::Release);
            self.inner.transformer_waker.wake();
        } else {
            self.inner.consumers_finished.store(0, Ordering::Relaxed);
            self.inner.state.store(State::Transformed as u8, Ordering::Release);
            #[cfg(not(feature = "alloc"))]
            for i in 0..registration.consumer_count() as usize {
                self.inner.consumer_wakers[i].wake();
            }
            #[cfg(feature = "alloc")]
            {
                let wakers = self.inner.consumer_wakers.lock().unwrap();
                for waker in wakers.iter() {
                    waker.wake();
                }
            }
        }
    }
}

impl<'a, S: Storage<Item = u8> + 'a> Transformer<'a> for TransformerHandle<Slot<S>> {
    async fn acquire_transform(&'a self) -> TransformPayload<'a, Self> {
        let registration: Registration = self.inner.registered.load(Ordering::Relaxed).into();
        assert!(registration.transformer(), "acquire_transform called on a Slot configured without a transformer");

        poll_fn(|cx| {
            if self.inner.state.compare_exchange(
                State::Full as u8, State::Transforming as u8, Ordering::Acquire, Ordering::Relaxed
            ).is_ok() {
                let (buffer, metadata) = unsafe {
                    let buffer_mut = self.inner.storage.slice_mut(0..self.inner.storage.len());
                    let buffer_ready = &mut *(buffer_mut as *mut [MaybeUninit<u8>] as *mut [u8]);
                    let metadata_ref = (*self.inner.payload_metadata.get()).unwrap();
                    (buffer_ready, metadata_ref)
                };
                Poll::Ready(TransformPayload::new(buffer, metadata, self))
            } else {
                self.inner.transformer_waker.register(cx.waker());
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
        self.inner.consumers_finished.store(0, Ordering::Relaxed);
        self.inner.state.store(State::Transformed as u8, Ordering::Release);
        
        #[cfg(not(feature = "alloc"))] {
            let registration: Registration = self.inner.registered.load(Ordering::Relaxed).into();
            for i in 0..registration.consumer_count() as usize {
                self.inner.consumer_wakers[i].wake();
            }
        }
        #[cfg(feature = "alloc")] {
            let wakers = self.inner.consumer_wakers.lock().unwrap();
            for waker in wakers.iter() {
                waker.wake();
            }
        }
    }
}

impl<'a, S: Storage<Item = u8> + 'a> Consumer<'a> for ConsumerHandle<Slot<S>> {
    async fn acquire_read(&'a self) -> ReadPayload<'a, Self> {
        poll_fn(|cx| {
            let cur_cons_bit: u8 = 1 << self.id;
            if self.inner.consumers_finished.load(Ordering::Relaxed) & cur_cons_bit != 0 {
                 #[cfg(not(feature = "alloc"))]
                self.inner.consumer_wakers[self.id as usize].register(cx.waker());
                #[cfg(feature = "alloc")]
                {
                    let wakers = self.inner.consumer_wakers.lock().unwrap();
                    if let Some(waker) = wakers.get(self.id as usize) {
                        waker.register(cx.waker());
                    }
                }
                return Poll::Pending;
            }

            if self.inner.state.load(Ordering::Acquire) == State::Transformed as u8 {
                let (buffer, metadata) = unsafe {
                    let buffer_slice = self.inner.storage.slice_mut(0..self.inner.storage.len());
                    let buffer_ref = &*(buffer_slice as *const [MaybeUninit<u8>] as *const [u8]);
                    let metadata_ref = (*self.inner.payload_metadata.get()).unwrap();
                    (buffer_ref, metadata_ref)
                };
                Poll::Ready(ReadPayload::new(buffer, metadata, self))
            } else {
                 #[cfg(not(feature = "alloc"))]
                self.inner.consumer_wakers[self.id as usize].register(cx.waker());
                #[cfg(feature = "alloc")]
                {
                    let wakers = self.inner.consumer_wakers.lock().unwrap();
                    if let Some(waker) = wakers.get(self.id as usize) {
                        waker.register(cx.waker());
                    }
                }
                Poll::Pending
            }
        }).await
    }

    fn release_read(&self, remaining_length: usize) {
        trace!("Consumer {} releasing read slot", self.id);
        if remaining_length > 0 {
            panic!("Consumer must consume all data in the slot. Partial consumption is not supported in Slot.");
        }

        let cur_cons_bit = 1 << self.id;
        let finished_mask = self.inner.consumers_finished.fetch_or(cur_cons_bit, Ordering::AcqRel);

        let new_mask = finished_mask | cur_cons_bit;
        
        let registration: Registration = self.inner.registered.load(Ordering::Relaxed).into();
        let consumer_count = registration.consumer_count();
        let registered_mask = (1 << consumer_count) - 1;
        
        if new_mask == registered_mask {
            trace!("Last consumer finished. Resetting slot.");
            self.inner.state.store(State::Empty as u8, Ordering::Release);
            self.inner.producer_waker.wake();
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
        let slot = Arc::new(slot);
        
        let producer = ProducerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });
        let consumer = ConsumerHandle::new(slot.clone(), PayloadSize { preferred: 4, min: 4 });

        let consumer_handle = tokio::spawn(async move {
            let payload = consumer.acquire_read().await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
            consumer.release_read(0);
        });

        let mut write_payload = producer.acquire_write().await;
        assert_eq!(get_current_state(&slot), State::Writing);
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.set_valid_length(4);
        
        drop(write_payload);
        assert_eq!(get_current_state(&slot), State::Transformed);

        timeout(Duration::from_millis(100), consumer_handle).await.expect("Consumer timed out").unwrap();
        
        assert_eq!(get_current_state(&slot), State::Empty);
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
            let payload = consumer.acquire_read().await;
            assert_eq!(payload.len(), 4);
            assert_eq!(&payload[..], &[11, 12, 13, 14]);
            consumer.release_read(0);
        });
        
        let transformer_handle = tokio::spawn(async move {
            let mut payload = transformer.acquire_transform().await;
            assert_eq!(&payload[..], &[1, 2, 3, 4]);
            for byte in payload.iter_mut() {
                *byte += 10;
            }
            transformer.release_transform(payload.metadata, 0);
        });

        let mut write_payload = producer.acquire_write().await;
        write_payload[0..4].copy_from_slice(&[1, 2, 3, 4]);
        write_payload.set_valid_length(4);
        
        drop(write_payload);
        assert_eq!(get_current_state(&slot), State::Full);

        timeout(Duration::from_millis(100), transformer_handle).await.expect("Transformer timed out").unwrap();
        
        assert_eq!(get_current_state(&slot), State::Transformed);

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
            consumer.acquire_read().await;
        });

        tokio::task::yield_now().await;

        let mut write_payload = producer.acquire_write().await;
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
                let mut write_payload = producer.acquire_write().await;
                write_payload[0..4].copy_from_slice(&[10, 20, 30, 40]);
                write_payload.set_valid_length(4);
            }
        });

        producer_handle.await.unwrap();
        assert_eq!(get_current_state(&slot), State::Transformed);

        let (c1_tx, c1_rx) = oneshot::channel();

        let consumer1_handle = tokio::spawn(async move {
            let payload = consumer1.acquire_read().await;
            assert_eq!(&payload[..], &[10, 20, 30, 40]);
            c1_tx.send(()).unwrap();
            consumer1.release_read(0);
        });

        let consumer2_handle = tokio::spawn(async move {
            c1_rx.await.unwrap();
            let payload = consumer2.acquire_read().await;
            assert_eq!(&payload[..], &[10, 20, 30, 40]);
            consumer2.release_read(0);
        });

        consumer1_handle.await.unwrap();

        assert_ne!(get_current_state(&slot), State::Empty);

        consumer2_handle.await.unwrap();

        assert_eq!(get_current_state(&slot), State::Empty);
    }
}
