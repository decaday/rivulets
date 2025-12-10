use core::slice;
use core::mem::align_of;
use fundsp::prelude::*;

/// A helper to split input and output buffers into scalar edges and a SIMD body.
///
/// It handles two levels of alignment requirements:
/// 1. **Memory Alignment**: Ensures `body` addresses are aligned to `F32x`.
/// 2. **Library Constraints**: Ensures `body` length is a multiple of the unroll factor (8),
///    which is required by `fundsp::BufferMut` to avoid panics.
pub struct SplitBuffer<'a> {
    /// Scalar samples at the beginning.
    pub head: (&'a [f32], &'a mut [f32]),
    /// Aligned and sized SIMD vectors in the middle (ready for Block Processing).
    pub body: (&'a [F32x], &'a mut [F32x]),
    /// Scalar samples at the end (including any leftovers that didn't fit in the SIMD block).
    pub tail: (&'a [f32], &'a mut [f32]),
}

impl<'a> SplitBuffer<'a> {
    pub fn new(input: &'a [f32], output: &'a mut [f32]) -> Self {
        assert_eq!(input.len(), output.len());

        // 1. Pre-check Memory Alignment
        // We calculate offsets first to avoid borrowing `output` mutably unless we are sure we want to split.
        // align_offset returns the number of elements to skip to reach alignment.
        // It returns usize::MAX if alignment is impossible.
        let in_ptr = input.as_ptr();
        let out_ptr = output.as_mut_ptr();

        let in_offset = in_ptr.align_offset(align_of::<F32x>());
        let out_offset = out_ptr.align_offset(align_of::<F32x>());

        // align_to() produces a head slice that ends at the aligned address.
        // Its length is min(offset, len).
        let in_head_len = core::cmp::min(in_offset, input.len());
        let out_head_len = core::cmp::min(out_offset, output.len());

        // Only proceed with SIMD split if alignment boundaries match.
        if in_head_len == out_head_len {
            // Safety: f32 and F32x are POD types. We confirmed alignments match.
            // Now we can safely borrow output.
            let (in_head, in_body, in_tail) = unsafe { input.align_to::<F32x>() };
            let (out_head, out_body, out_tail) = unsafe { output.align_to_mut::<F32x>() };

            // 2. Library Constraint (Unroll Factor)
            // fundsp requires buffer length to be a multiple of 8.
            const UNROLL_MASK: usize = 7; // 8 - 1
            
            let valid_vectors = in_body.len() & !UNROLL_MASK;
            
            // If we have "leftover" vectors that are aligned but don't fill an unroll block:
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
                // Since `align_to` ensures Body is immediately followed by Tail in memory,
                // we can construct a new Tail slice that starts earlier.
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
                // Perfect fit
                Self {
                    head: (in_head, out_head),
                    body: (in_body, out_body),
                    tail: (in_tail, out_tail),
                }
            }
        } else {
            // Alignment mismatch fallback
            // Since we didn't call align_to_mut yet, output is not borrowed, so this is safe.
            Self {
                head: (input, output),
                body: (&[], &mut []),
                tail: (&[], &mut []),
            }
        }
    }
}
