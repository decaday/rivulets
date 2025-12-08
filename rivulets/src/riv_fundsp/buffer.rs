use fundsp::buffer::{BufferRef, BufferMut};
use fundsp::F32x;
use rivulets_driver::databus::{Consumer, Producer};
use rivulets_driver::payload::{ReadPayload, WritePayload};
use std::mem::size_of;
use std::ops::{Deref, DerefMut};
use std::slice;

// The number of f32 samples in one F32x SIMD vector.
const F32_PER_F32X: usize = size_of::<F32x>() / size_of::<f32>();
// The maximum number of f32 samples allowed in a single processing chunk.
const MAX_SAMPLES_PER_CHUNK: usize = 64;
// The size of a processing chunk in terms of F32x vectors.
const CHUNK_SIZE_IN_F32X: usize = MAX_SAMPLES_PER_CHUNK / F32_PER_F32X;

/// A read-only view of a payload, interpreted as a slice of SIMD vectors.
pub struct BufferReadPayload<'a> {
    data: &'a [F32x],
}

impl<'a> BufferReadPayload<'a> {
    /// Creates a SIMD view from a reference to a `ReadPayload`.
    ///
    /// # Panics
    /// Panics if the payload's length is not a multiple of the `F32x` vector size,
    /// or if its memory alignment is not suitable for `F32x`.
    pub fn new<C: Consumer>(payload: &'a ReadPayload<'a, C>) -> Self {
        let data_bytes: &[u8] = payload.deref();
        assert_eq!(data_bytes.len() % size_of::<f32>(), 0, "Data length must be a multiple of f32 size.");
        assert_ne!(F32_PER_F32X, 0, "F32x cannot be zero-sized.");

        let data_f32_len = data_bytes.len() / size_of::<f32>();
        assert_eq!(data_f32_len % F32_PER_F32X, 0, "Data length must be a multiple of the SIMD vector size.");

        let data_f32x_len = data_f32_len / F32_PER_F32X;
        let data = unsafe {
            slice::from_raw_parts(data_bytes.as_ptr() as *const F32x, data_f32x_len)
        };

        Self { data }
    }

    /// Returns an iterator that yields `BufferRef` for each chunk.
    pub fn chunks(&'a self) -> BufferRefChunks<'a> {
        BufferRefChunks {
            chunks: self.data.chunks(CHUNK_SIZE_IN_F32X),
        }
    }
}

/// An iterator that yields `BufferRef`s over data chunks.
pub struct BufferRefChunks<'a> {
    chunks: slice::Chunks<'a, F32x>,
}

impl<'a> Iterator for BufferRefChunks<'a> {
    type Item = BufferRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().map(BufferRef::new)
    }
}

/// A mutable view of a payload, interpreted as a slice of SIMD vectors.
pub struct BufferWritePayload<'a> {
    data: &'a mut [F32x],
}

impl<'a> BufferWritePayload<'a> {
    /// Creates a mutable SIMD view from a mutable reference to a `WritePayload`.
    ///
    /// # Panics
    /// Panics if the payload's length is not a multiple of the `F32x` vector size,
    /// or if its memory alignment is not suitable for `F32x`.
    pub fn new<P: Producer>(payload: &'a mut WritePayload<'a, P>) -> Self {
        let data_bytes: &mut [u8] = payload.deref_mut();
        assert_eq!(data_bytes.len() % size_of::<f32>(), 0, "Data length must be a multiple of f32 size.");
        assert_ne!(F32_PER_F32X, 0, "F32x cannot be zero-sized.");
        
        let data_f32_len = data_bytes.len() / size_of::<f32>();
        assert_eq!(data_f32_len % F32_PER_F32X, 0, "Data length must be a multiple of the SIMD vector size.");

        let data_f32x_len = data_f32_len / F32_PER_F32X;
        let data = unsafe {
            slice::from_raw_parts_mut(data_bytes.as_mut_ptr() as *mut F32x, data_f32x_len)
        };

        Self { data }
    }

    /// Returns an iterator that yields `BufferMut` for each chunk.
    pub fn chunks_mut(&'a mut self) -> BufferMutChunks<'a> {
        BufferMutChunks {
            chunks: self.data.chunks_mut(CHUNK_SIZE_IN_F32X),
        }
    }
}

/// An iterator that yields `BufferMut`s over data chunks.
pub struct BufferMutChunks<'a> {
    chunks: slice::ChunksMut<'a, F32x>,
}

impl<'a> Iterator for BufferMutChunks<'a> {
    type Item = BufferMut<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().map(BufferMut::new)
    }
}

/// A view of a readable and a writable payload, interpreted as slices of f32 samples.
/// This is used when SIMD alignment for F32x cannot be guaranteed.
pub struct BufferPayload<'a> {
    /// The read-only slice of f32 samples.
    pub read: &'a [f32],
    /// The mutable slice of f32 samples.
    pub write: &'a mut [f32],
}

impl<'a> BufferPayload<'a> {
    /// Creates a new `BufferPayload` from a `ReadPayload` and a `WritePayload`.
    ///
    /// # Panics
    /// Panics if the payloads have different lengths or if their lengths are not a multiple of the f32 size.
    pub fn new<C: Consumer, P: Producer>(
        read_payload: &ReadPayload<'a, C>,
        write_payload: &mut WritePayload<'a, P>,
    ) -> Self {
        let read_bytes: &[u8] = read_payload.deref();
        let write_bytes: &mut [u8] = write_payload.deref_mut();

        assert_eq!(
            read_bytes.len(),
            write_bytes.len(),
            "Read and write payloads must have the same length."
        );
        assert_eq!(
            read_bytes.len() % size_of::<f32>(),
            0,
            "Payload length must be a multiple of f32 size."
        );

        let len_in_f32 = read_bytes.len() / size_of::<f32>();

        // Safety: We have asserted that the byte slice length is a multiple of the f32 size.
        // The pointers are derived from valid slices. The lifetime 'a is carried over correctly.
        let read_slice = unsafe {
            slice::from_raw_parts(read_bytes.as_ptr() as *const f32, len_in_f32)
        };

        // Safety: Same reasons as above, but for a mutable slice.
        let write_slice = unsafe {
            slice::from_raw_parts_mut(write_bytes.as_mut_ptr() as *mut f32, len_in_f32)
        };

        Self {
            read: read_slice,
            write: write_slice,
        }
    }
}

impl<'a> BufferPayload<'a> {
    /// Returns an iterator that yields pairs of (read_sample, write_sample_mut).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&f32, &mut f32)> {
        (*self.read).iter().zip(self.write.iter_mut())
    }
}

