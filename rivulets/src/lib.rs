#![cfg_attr(not(feature = "std"), no_std)]

pub mod fmt;

pub mod utils;
pub mod nodes;
pub mod elements;
pub mod databus;
#[cfg(feature="fundsp")]
pub mod riv_fundsp;

pub use ringbuf::storage;

// pub mod transformer;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        pub use std::sync::Mutex;
        
        pub use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as RawMutex;
    } else {
        pub type Mutex<T> = embassy_sync::blocking_mutex::CriticalSectionMutex<T>;
        
        pub use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as RawMutex;
    }
}

/// A standard configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardConfig {
    /// The preferred batch size for processing loop.
    pub prefer_items_per_process: u16,
}

impl Default for StandardConfig {
    fn default() -> Self {
        Self {
            prefer_items_per_process: 64,
        }
    }
}

impl StandardConfig {
    pub fn new(prefer_items_per_process: u16) -> Self {
        Self { prefer_items_per_process }
    }
}