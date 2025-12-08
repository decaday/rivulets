#![cfg_attr(not(feature = "std"), no_std)]

pub mod stream;
pub mod element;
pub mod port;
pub mod info;
pub mod databus;
pub mod payload;
pub mod node;

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