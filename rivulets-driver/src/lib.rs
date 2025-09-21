#![cfg_attr(not(feature = "std"), no_std)]

pub mod stream;
pub mod element;
pub mod port;
pub mod info;
pub mod databus;
pub mod payload;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        pub use std::sync::Mutex;
        pub use std::sync::MutexGuard;
    } else {
        pub type Mutex<T> = embassy_sync::blocking_mutex::CriticalSectionMutex<T>;
    }
}

#[derive(Debug)]
pub enum Error {
    /// Invalid parameters provided.
    InvalidParameter,
    /// Device is not initialized.
    NotInitialized,
    /// Device is in an invalid state for the requested operation.
    InvalidState,
    /// Device is busy.
    Busy,
    /// Operation timed out.
    Timeout,
    /// Buffer is full.
    BufferFull,
    /// Buffer is empty.
    BufferEmpty,
    /// Device hardware error.
    DeviceError,
    /// Operation not supported.
    Unsupported,
}