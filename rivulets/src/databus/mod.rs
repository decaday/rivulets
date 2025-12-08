use rivulets_driver::databus::{Databus, DatabusRef};
use rivulets_driver::port::PayloadSize;

pub mod slot;

pub use handles::{ConsumerHandle, ProducerHandle, TransformerHandle};
pub use register::{ParticipantRegistry, Registration};

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
        pub fn new(databus: P, payload_size: PayloadSize) -> Self {
            databus.do_register_producer(payload_size);
            Self { inner: databus }
        }
    }

    impl<D: Databus, P: DatabusRef<Databus=D>> ConsumerHandle<D, P> {
        pub fn new(databus: P, payload_size: PayloadSize) -> Self {
            let id = databus.do_register_consumer(payload_size);
            Self { inner: databus, id }
        }
    }

    impl<D: Databus, P: DatabusRef<Databus=D>> TransformerHandle<D, P> {
        pub fn new(databus: P, payload_size: PayloadSize) -> Self {
            databus.do_register_transformer(payload_size);
            Self { inner: databus }
        }
    }
}

pub mod register {
    use core::cell::UnsafeCell;
    use core::marker::PhantomData;
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

    /// Bitfield representation of participant registrations.
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
    /// This struct is generic over the state that a specific databus implementation
    /// might need to associate with each participant.
    ///
    /// Generic Parameters:
    /// - `PS`: The databus-specific, thread-safe state for the Producer.
    /// - `CS`: The databus-specific, thread-safe state for each Consumer.
    /// - `TS`: The databus-specific, thread-safe state for the Transformer.
    pub struct ParticipantRegistry<PS: Default, CS: Default, TS: Default> {
        // UnsafeCell allows for interior mutability of initialized participant data.
        producer: UnsafeCell<(AtomicWaker, PS)>,
        transformer: UnsafeCell<(AtomicWaker, TS)>,

        #[cfg(not(feature = "alloc"))]
        consumers: [UnsafeCell<(AtomicWaker, CS)>; MAX_CONSUMERS],
        #[cfg(feature = "alloc")]
        consumers: Mutex<Vec<Box<(AtomicWaker, CS)>>>,

        registered: AtomicU8,
        consumers_finished: AtomicU8,
        _phantom: PhantomData<(PS, CS, TS)>,
    }

    // The registry is Sync if the associated states are Sync.
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
                _phantom: PhantomData,
            }
        }

        /// Gets a snapshot of the current registrations.
        pub fn registration(&self) -> Registration {
            self.registered.load(Ordering::Relaxed).into()
        }

        // --- Registration API ---

        pub fn register_producer(&self, initial_state: PS) {
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
            // SAFETY: This is safe because the atomic CAS loop above ensures this write
            // happens only once, preventing race conditions.
            unsafe {
                *self.producer.get() = (AtomicWaker::new(), initial_state);
            }
        }

        pub fn register_consumer(&self, initial_state: CS) -> u8 {
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

            #[cfg(not(feature = "alloc"))]
            {
                // SAFETY: This is safe because the atomic CAS loop guarantees that each consumer
                // gets a unique ID, preventing concurrent writes to the same array index.
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
            // SAFETY: The caller must ensure registration is complete before calling this,
            // as the waker instance could be replaced during registration.
            unsafe {
                (*self.producer.get()).0.register(waker);
            }
        }

        pub fn register_transformer_waker(&self, waker: &Waker) {
            // SAFETY: See safety comment in `register_producer_waker`.
            unsafe {
                (*self.transformer.get()).0.register(waker);
            }
        }

        pub fn register_consumer_waker(&self, id: u8, waker: &Waker) {
            // SAFETY: See safety comment in `register_producer_waker`.
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
            // SAFETY: The caller must ensure the participant is registered.
            unsafe {
                (*self.producer.get()).0.wake();
            }
        }

        pub fn wake_transformer(&self) {
            // SAFETY: The caller must ensure the participant is registered.
            unsafe {
                (*self.transformer.get()).0.wake();
            }
        }

        pub fn wake_all_consumers(&self) {
            #[cfg(not(feature = "alloc"))]
            for i in 0..self.registration().consumer_count() as usize {
                // SAFETY: The caller must ensure the participant is registered.
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

        /// Marks a consumer as finished.
        ///
        /// Returns `true` if this was the last registered consumer to finish.
        pub fn mark_consumer_as_finished(&self, id: u8) -> bool {
            let cur_cons_bit = 1u8 << id;
            // fetch_or returns the value *before* the operation.
            let previous_mask = self.consumers_finished.fetch_or(cur_cons_bit, Ordering::AcqRel);
            let new_mask = previous_mask | cur_cons_bit;

            let registration = self.registration();
            let consumer_count = registration.consumer_count();
            // The required mask is a bitmask with `consumer_count` bits set to 1.
            let required_mask = if consumer_count > 0 { (1u16 << consumer_count) - 1 } else { 0 } as u8;

            new_mask == required_mask && required_mask != 0
        }

        /// Checks if a specific consumer has already been marked as finished.
        pub fn is_consumer_finished(&self, id: u8) -> bool {
            let cur_cons_bit = 1u8 << id;
            self.consumers_finished.load(Ordering::Relaxed) & cur_cons_bit != 0
        }

        /// Resets the finished status for all consumers.
        pub fn reset_consumers_finished(&self) {
            self.consumers_finished.store(0, Ordering::Relaxed);
        }

        // --- State Access API ---

        /// Gets an immutable reference to the producer's state.
        ///
        /// # Safety
        /// The caller must ensure no other thread is concurrently modifying the state.
        /// This is typically guaranteed by the databus's state machine.
        pub unsafe fn producer_state(&self) -> &PS {
            &(*self.producer.get()).1
        }
        
        /// Gets a mutable reference to the producer's state.
        ///
        /// # Safety
        /// The caller must ensure no other thread is concurrently accessing the state.
        pub unsafe fn producer_state_mut(&self) -> &mut PS {
            &mut (*self.producer.get()).1
        }

        /// Gets an immutable reference to a specific consumer's state.
        ///
        /// # Safety
        /// See safety comment in `producer_state`. On `alloc` builds, the returned
        /// reference is valid because the consumer states are boxed and their addresses
        /// remain stable even if the vector reallocates.
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
        
        /// Gets a mutable reference to a specific consumer's state.
        ///
        /// # Safety
        /// See safety comments in `producer_state_mut` and `consumer_state`.
        pub unsafe fn consumer_state_mut(&self, id: u8) -> &mut CS {
            #[cfg(not(feature = "alloc"))]
            {
                &mut (*self.consumers[id as usize].get()).1
            }
            #[cfg(feature = "alloc")]
            {
                let guard = self.consumers.lock().unwrap();
                let item = &guard[id as usize];
                let ptr = &**item as *const (AtomicWaker, CS) as *mut (AtomicWaker, CS);
                &mut (*ptr).1
            }
        }

        /// Gets an immutable reference to the transformer's state.
        ///
        /// # Safety
        /// See safety comment in `producer_state`.
        pub unsafe fn transformer_state(&self) -> &TS {
            &(*self.transformer.get()).1
        }

        /// Gets a mutable reference to the transformer's state.
        ///
        /// # Safety
        /// See safety comment in `producer_state_mut`.
        pub unsafe fn transformer_state_mut(&self) -> &mut TS {
            &mut (*self.transformer.get()).1
        }
    }

    impl<PS: Default, CS: Default, TS: Default> Default for ParticipantRegistry<PS, CS, TS> {
        fn default() -> Self {
            Self::new()
        }
    }
}
