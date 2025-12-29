use rivulets_driver::databus::{Databus, DatabusRef};
use rivulets_driver::port::{PayloadSize, PortRequirements};

pub mod slot;
pub mod ring_buffer;

pub use handles::{ConsumerHandle, ProducerHandle, TransformerHandle};
pub use registry::{ParticipantRegistry, Registration};

mod handles {
    use super::*;
    pub struct ProducerHandle<D: Databus, P: DatabusRef<Databus=D>> {
        pub inner: P,
    }

    pub struct ConsumerHandle<D: Databus, P: DatabusRef<Databus=D>> {
        pub inner: P,
        pub id: u8,
    }

    pub struct TransformerHandle<D: Databus, P: DatabusRef<Databus=D>> {
        pub inner: P,
    }

    impl<D: Databus, P: DatabusRef<Databus=D>> ProducerHandle<D, P> {
        pub fn new(databus: P, payload_size: PayloadSize, strict_alloc: bool) -> Self {
            databus.do_register_producer(payload_size, strict_alloc);
            Self { inner: databus }
        }

        pub fn from_port_requirements(databus: P, reqs: &PortRequirements) -> Option<Self> {
            if let Some(payload_size) = reqs.out {
                Some(Self::new(databus, payload_size, reqs.strict_alloc))
            } else {
                None
            }
        }
    }

    impl<D: Databus, P: DatabusRef<Databus=D>> ConsumerHandle<D, P> {
        pub fn new(databus: P, payload_size: PayloadSize, consume_all: bool) -> Self {
            let id = databus.do_register_consumer(payload_size, consume_all);
            Self { inner: databus, id }
        }

        pub fn from_port_requirements(databus: P, reqs: &PortRequirements) -> Option<Self> {
            if let Some(payload_size) = reqs.in_ {
                Some(Self::new(databus, payload_size, reqs.consume_all))
            } else {
                None
            }
        }
    }

    impl<D: Databus, P: DatabusRef<Databus=D>> TransformerHandle<D, P> {
        pub fn new(databus: P, payload_size: PayloadSize, strict_alloc: bool, consume_all: bool) -> Self {
            databus.do_register_transformer(payload_size, strict_alloc, consume_all);
            Self { inner: databus }
        }

        pub fn from_port_requirements(databus: P, reqs: &PortRequirements) -> Option<Self> {
            if let Some(payload_size) = reqs.out {
                if reqs.in_place {
                    Some(Self::new(databus, payload_size, reqs.strict_alloc, reqs.consume_all))
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

pub mod registry {
    use core::cell::UnsafeCell;
    use core::task::Waker;
    use embassy_sync::waitqueue::AtomicWaker;
    use portable_atomic::{AtomicU8, Ordering};

    use bitfield_struct::bitfield;

    #[cfg(feature = "alloc")]
    extern crate alloc;
    #[cfg(feature = "alloc")]
    use alloc::vec::Vec;
    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;
    #[cfg(feature = "alloc")]
    use crate::Mutex;

    #[cfg(not(feature = "alloc"))]
    const MAX_CONSUMERS: usize = 2;
    #[cfg(feature = "alloc")]
    const MAX_CONSUMERS: usize = 8;

    #[bitfield(u8)]
    #[derive(PartialEq, Eq)]
    pub struct Registration {
        pub producer: bool,
        pub transformer: bool,
        #[bits(6)]
        pub consumer_count: u8,
    }

    /// A manager for participant registration, state, and wakers.
    ///
    /// Generic Parameters:
    /// - `PS`: The databus-specific, thread-safe state for the Producer.
    /// - `CS`: The databus-specific, thread-safe state for each Consumer.
    /// - `TS`: The databus-specific, thread-safe state for the Transformer.
    pub struct ParticipantRegistry<PS: Default, CS: Default, TS: Default> {
        producer: UnsafeCell<(AtomicWaker, PS)>,
        transformer: UnsafeCell<(AtomicWaker, TS)>,

        #[cfg(not(feature = "alloc"))]
        consumers: [UnsafeCell<(AtomicWaker, CS)>; MAX_CONSUMERS],
        #[cfg(feature = "alloc")]
        consumers: Mutex<Vec<Box<(AtomicWaker, CS)>>>,

        registered: AtomicU8,
        consumers_finished: AtomicU8,
    }

    unsafe impl<PS: Default + Sync, CS: Default + Sync, TS: Default + Sync> Sync for ParticipantRegistry<PS, CS, TS> {}

    impl<PS: Default, CS: Default, TS: Default> ParticipantRegistry<PS, CS, TS> {
        pub fn new() -> Self {
            Self {
                producer: UnsafeCell::new((AtomicWaker::new(), PS::default())),
                transformer: UnsafeCell::new((AtomicWaker::new(), TS::default())),
                #[cfg(not(feature = "alloc"))]
                consumers: core::array::from_fn(|_| UnsafeCell::new((AtomicWaker::new(), CS::default()))),
                #[cfg(feature = "alloc")]
                consumers: Mutex::new(Vec::new()),
                registered: AtomicU8::new(Registration::new().into()),
                consumers_finished: AtomicU8::new(0),
            }
        }

        pub fn registration(&self) -> Registration {
            self.registered.load(Ordering::Relaxed).into()
        }

        // --- Registration API ---

        pub fn register_producer(&self, initial_state: PS) {
            let _ = self.registered.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |val| {
                let reg = Registration::from(val);
                if reg.producer() {
                    None
                } else {
                    Some(reg.with_producer(true).into())
                }
            }).expect("Producer already registered");

            // SAFETY: fetch_update guarantees this block runs exactly once, preventing race conditions.
            unsafe {
                *self.producer.get() = (AtomicWaker::new(), initial_state);
            }
        }

        pub fn register_consumer(&self, initial_state: CS) -> u8 {
            let prev = self.registered.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |val| {
                let reg = Registration::from(val);
                if reg.consumer_count() as usize >= MAX_CONSUMERS {
                    None
                } else {
                    Some(reg.with_consumer_count(reg.consumer_count() + 1).into())
                }
            }).expect("Maximum number of consumers reached");

            let id = Registration::from(prev).consumer_count();

            #[cfg(not(feature = "alloc"))]
            {
                // SAFETY: Atomic count increment guarantees a unique ID for each consumer.
                unsafe {
                    *self.consumers[id as usize].get() = (AtomicWaker::new(), initial_state);
                }
            }
            #[cfg(feature = "alloc")]
            {
                let mut wakers = self.consumers.lock().unwrap();
                wakers.push(Box::new((AtomicWaker::new(), initial_state)));
            }
            id
        }

        pub fn register_transformer(&self, initial_state: TS) {
            let _ = self.registered.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |val| {
                let reg = Registration::from(val);
                if reg.transformer() {
                    None
                } else {
                    Some(reg.with_transformer(true).into())
                }
            }).expect("Transformer already registered");

            // SAFETY: See safety comment in `register_producer`.
            unsafe {
                *self.transformer.get() = (AtomicWaker::new(), initial_state);
            }
        }

        // --- Pre-condition Checks ---

        pub fn acquire_write_check(&self) {
            assert!(self.registration().producer(), "acquire_write called on a Slot configured without a producer");
        }

        pub fn acquire_read_check(&self) {
            assert!(self.registration().consumer_count() > 0, "acquire_read called on a Slot configured without any consumers");
        }

        pub fn acquire_transform_check(&self) {
            assert!(self.registration().transformer(), "acquire_transform called on a Slot configured without a transformer");
        }

        // --- Waker API ---

        pub fn register_producer_waker(&self, waker: &Waker) {
            unsafe {
                (*self.producer.get()).0.register(waker);
            }
        }

        pub fn register_transformer_waker(&self, waker: &Waker) {
            unsafe {
                (*self.transformer.get()).0.register(waker);
            }
        }

        pub fn register_consumer_waker(&self, id: u8, waker: &Waker) {
            #[cfg(not(feature = "alloc"))]
            unsafe {
                (*self.consumers[id as usize].get()).0.register(waker);
            }
            #[cfg(feature = "alloc")]
            {
                let wakers = self.consumers.lock().unwrap();
                wakers[id as usize].0.register(waker);
            }
        }

        pub fn wake_producer(&self) {
            unsafe {
                (*self.producer.get()).0.wake();
            }
        }

        pub fn wake_transformer(&self) {
            unsafe {
                (*self.transformer.get()).0.wake();
            }
        }

        pub fn wake_all_consumers(&self) {
            #[cfg(not(feature = "alloc"))]
            for i in 0..self.registration().consumer_count() as usize {
                unsafe {
                    (*self.consumers[i].get()).0.wake();
                }
            }
            #[cfg(feature = "alloc")]
            {
                let wakers = self.consumers.lock().unwrap();
                for waker in wakers.iter() {
                    waker.0.wake();
                }
            }
        }

        // --- Consumer Finished State API ---

        pub fn mark_consumer_as_finished(&self, id: u8) -> bool {
            let cur_cons_bit = 1u8 << id;
            let previous_mask = self.consumers_finished.fetch_or(cur_cons_bit, Ordering::AcqRel);
            let new_mask = previous_mask | cur_cons_bit;

            let registration = self.registration();
            let consumer_count = registration.consumer_count();
            let required_mask = if consumer_count > 0 { (1u16 << consumer_count) - 1 } else { 0 } as u8;

            new_mask == required_mask && required_mask != 0
        }

        pub fn is_consumer_finished(&self, id: u8) -> bool {
            let cur_cons_bit = 1u8 << id;
            self.consumers_finished.load(Ordering::Relaxed) & cur_cons_bit != 0
        }

        pub fn reset_consumers_finished(&self) {
            self.consumers_finished.store(0, Ordering::Relaxed);
        }

        // --- State Access API ---

        /// Gets an immutable reference to the producer's state.
        ///
        /// # Safety
        /// The caller must ensure that the producer has been registered and initialized.
        /// Concurrent modification is handled by the `Sync` requirement of `PS` (e.g., using `Atomic` or `Mutex` types).
        pub unsafe fn producer_state(&self) -> &PS {
            &(*self.producer.get()).1
        }

        /// Gets an immutable reference to a specific consumer's state.
        ///
        /// # Safety
        /// The caller must ensure that the consumer with `id` has been registered.
        /// Access is thread-safe because `CS` implements `Sync`.
        pub unsafe fn consumer_state(&self, id: u8) -> &CS {
            #[cfg(not(feature = "alloc"))]
            {
                &(*self.consumers[id as usize].get()).1
            }
            #[cfg(feature = "alloc")]
            {
                let guard = self.consumers.lock().unwrap();
                let item = &guard[id as usize];
                let ptr = &**item as *const (AtomicWaker, CS);
                &(*ptr).1
            }
        }

        /// Gets an immutable reference to the transformer's state.
        ///
        /// # Safety
        /// The caller must ensure that the transformer has been registered.
        /// Concurrent modification is handled by the `Sync` requirement of `TS`.
        pub unsafe fn transformer_state(&self) -> &TS {
            &(*self.transformer.get()).1
        }
    }

    impl<PS: Default, CS: Default, TS: Default> Default for ParticipantRegistry<PS, CS, TS> {
        fn default() -> Self {
            Self::new()
        }
    }
}
