use core::slice;
use core::mem::align_of;
use fundsp::prelude::*;
use fundsp::buffer::{BufferMut, BufferRef};

/// A helper to split input and output buffers into scalar edges and a SIMD body.
///
/// It handles two levels of alignment requirements:
/// 1. **Memory Alignment**: Ensures `body` addresses are aligned to `F32x`.
/// 2. **Library Constraints**: Ensures `body` length is a multiple of the unroll factor (8),
///    which is required by `fundsp::BufferMut` to avoid panics.
pub struct SplitBuffer<'a> {
    /// Scalar samples at the beginning.
    pub head: (&'a [f32], &'a mut [f32]),
    /// Aligned and sized SIMD vectors in the middle.
    pub body: (&'a [F32x], &'a mut [F32x]),
    /// Scalar samples at the end (including any leftovers that didn't fit in the SIMD block).
    pub tail: (&'a [f32], &'a mut [f32]),
}

impl<'a> SplitBuffer<'a> {
    pub fn new(input: &'a [f32], output: &'a mut [f32]) -> Self {
        assert_eq!(input.len(), output.len());

        // 1. Pre-check Memory Alignment
        let in_ptr = input.as_ptr();
        let out_ptr = output.as_mut_ptr();

        let in_offset = in_ptr.align_offset(align_of::<F32x>());
        let out_offset = out_ptr.align_offset(align_of::<F32x>());

        let in_head_len = core::cmp::min(in_offset, input.len());
        let out_head_len = core::cmp::min(out_offset, output.len());

        // Only proceed with SIMD split if alignment boundaries match.
        if in_head_len == out_head_len {
            // Safety: f32 and F32x are POD types. We confirmed alignments match.
            let (in_head, in_body, in_tail) = unsafe { input.align_to::<F32x>() };
            let (out_head, out_body, out_tail) = unsafe { output.align_to_mut::<F32x>() };

            // 2. Library Constraint (Unroll Factor)
            const UNROLL_MASK: usize = 7; 
            
            let valid_vectors = in_body.len() & !UNROLL_MASK;
            
            if valid_vectors != in_body.len() {
                // Split the body: [Fast Path | Leftovers]
                let (fast_body_in, leftover_body_in) = in_body.split_at(valid_vectors);
                let (fast_body_out, leftover_body_out) = out_body.split_at_mut(valid_vectors);

                // Convert leftovers back to scalar slices
                // Safety: Reinterpreting generic POD slices is safe.
                let leftover_in_scalar = unsafe {
                    slice::from_raw_parts(
                        leftover_body_in.as_ptr() as *const f32,
                        leftover_body_in.len() * fundsp::SIMD_LEN
                    )
                };
                let leftover_out_scalar = unsafe {
                    slice::from_raw_parts_mut(
                        leftover_body_out.as_mut_ptr() as *mut f32,
                        leftover_body_out.len() * fundsp::SIMD_LEN
                    )
                };

                // Merge leftovers into Tail
                let new_tail_in_len = leftover_in_scalar.len() + in_tail.len();
                let new_tail_out_len = leftover_out_scalar.len() + out_tail.len();

                // Safety: We are reconstructing a valid slice over contiguous memory we already borrow.
                let new_tail_in = unsafe {
                    slice::from_raw_parts(leftover_in_scalar.as_ptr(), new_tail_in_len)
                };
                let new_tail_out = unsafe {
                    slice::from_raw_parts_mut(leftover_out_scalar.as_mut_ptr(), new_tail_out_len)
                };

                Self {
                    head: (in_head, out_head),
                    body: (fast_body_in, fast_body_out),
                    tail: (new_tail_in, new_tail_out),
                }
            } else {
                Self {
                    head: (in_head, out_head),
                    body: (in_body, out_body),
                    tail: (in_tail, out_tail),
                }
            }
        } else {
            // Alignment mismatch fallback
            Self {
                head: (input, output),
                body: (&[], &mut []),
                tail: (&[], &mut []),
            }
        }
    }

    /// Returns the SIMD body ready for processing, if any.
    ///
    /// The return tuple contains:
    /// 1. `usize`: Total number of samples (not vectors) to process.
    /// 2. `BufferRef`: The input buffer wrapper.
    /// 3. `BufferMut`: The output buffer wrapper.
    pub fn simd_parts(&'_ mut self) -> Option<(usize, BufferRef<'_>, BufferMut<'_>)> {
        if self.body.0.is_empty() {
            None
        } else {
            let samples = self.body.0.len() * fundsp::SIMD_LEN;
            let input = BufferRef::new(self.body.0);
            let output = BufferMut::new(self.body.1);
            Some((samples, input, output))
        }
    }

    /// Iterates over scalar samples in the Head and Tail sections.
    pub fn scalar_pairs(&mut self) -> impl Iterator<Item = (&f32, &mut f32)> {
        self.head.0.iter().zip(self.head.1.iter_mut())
            .chain(self.tail.0.iter().zip(self.tail.1.iter_mut()))
    }
}

/// A helper to split an output buffer for Source elements (No Input).
pub struct SplitSourceBuffer<'a> {
    pub head: &'a mut [f32],
    pub body: &'a mut [F32x],
    pub tail: &'a mut [f32],
}

impl<'a> SplitSourceBuffer<'a> {
    pub fn new(output: &'a mut [f32]) -> Self {
        // 1. Alignment
        // Safety: align_to_mut is safe for POD types.
        let (head, body, tail) = unsafe { output.align_to_mut::<F32x>() };

        // 2. Library Constraint (Unroll Factor)
        const UNROLL_MASK: usize = 7;
        let valid_vectors = body.len() & !UNROLL_MASK;

        if valid_vectors != body.len() {
            let (fast_body, leftover_body) = body.split_at_mut(valid_vectors);

            // Convert leftovers back to scalar
            let leftover_scalar = unsafe {
                slice::from_raw_parts_mut(
                    leftover_body.as_mut_ptr() as *mut f32,
                    leftover_body.len() * fundsp::SIMD_LEN
                )
            };

            // Merge leftovers into Tail
            let new_tail_len = leftover_scalar.len() + tail.len();
            let new_tail = unsafe {
                slice::from_raw_parts_mut(leftover_scalar.as_mut_ptr(), new_tail_len)
            };

            Self {
                head,
                body: fast_body,
                tail: new_tail,
            }
        } else {
            Self { head, body, tail }
        }
    }

    /// Returns the SIMD body ready for processing.
    ///
    /// Provides an empty `BufferRef` for input, as sources do not consume data.
    pub fn simd_parts(&'_ mut self) -> Option<(usize, BufferRef<'_>, BufferMut<'_>)> {
        if self.body.is_empty() {
            None
        } else {
            let samples = self.body.len() * fundsp::SIMD_LEN;
            // Source process: input buffer is empty
            let input = BufferRef::new(&[]);
            let output = BufferMut::new(self.body);
            Some((samples, input, output))
        }
    }

    /// Iterates over mutable scalar samples in Head and Tail.
    pub fn scalar_parts(&mut self) -> impl Iterator<Item = &mut f32> {
        self.head.iter_mut().chain(self.tail.iter_mut())
    }
}

/// A helper to split an input buffer for Sink elements (No Output).
pub struct SplitSinkBuffer<'a> {
    pub head: &'a [f32],
    pub body: &'a [F32x],
    pub tail: &'a [f32],
}

impl<'a> SplitSinkBuffer<'a> {
    pub fn new(input: &'a [f32]) -> Self {
        // 1. Alignment
        let (head, body, tail) = unsafe { input.align_to::<F32x>() };

        // 2. Library Constraint (Unroll Factor)
        const UNROLL_MASK: usize = 7;
        let valid_vectors = body.len() & !UNROLL_MASK;

        if valid_vectors != body.len() {
            let (fast_body, leftover_body) = body.split_at(valid_vectors);

            // Convert leftovers back to scalar
            let leftover_scalar = unsafe {
                slice::from_raw_parts(
                    leftover_body.as_ptr() as *const f32,
                    leftover_body.len() * fundsp::SIMD_LEN
                )
            };

            // Merge leftovers into Tail
            let new_tail_len = leftover_scalar.len() + tail.len();
            let new_tail = unsafe {
                slice::from_raw_parts(leftover_scalar.as_ptr(), new_tail_len)
            };

            Self {
                head,
                body: fast_body,
                tail: new_tail,
            }
        } else {
            Self { head, body, tail }
        }
    }

    /// Returns the SIMD body ready for processing.
    ///
    /// Provides an empty `BufferMut` for output, as sinks do not produce data.
    pub fn simd_parts(&'_ mut self) -> Option<(usize, BufferRef<'_>, BufferMut<'_>)> {
        if self.body.is_empty() {
            None
        } else {
            let samples = self.body.len() * fundsp::SIMD_LEN;
            let input = BufferRef::new(self.body);
            // Output buffer is empty (0 channels)
            let output = BufferMut::new(&mut []);
            Some((samples, input, output))
        }
    }

    /// Iterates over scalar samples in Head and Tail.
    pub fn scalar_parts(&self) -> impl Iterator<Item = &f32> {
        self.head.iter().chain(self.tail.iter())
    }
}
