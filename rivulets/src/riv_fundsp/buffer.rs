use fundsp::prelude::*;

/// A helper to split input and output buffers into scalar edges and a SIMD body.
pub struct SplitBuffer<'a> {
    /// Scalar samples at the beginning (unaligned).
    pub head: (&'a [f32], &'a mut [f32]),
    /// Aligned SIMD vectors in the middle.
    pub body: (&'a [F32x], &'a mut [F32x]),
    /// Scalar samples at the end.
    pub tail: (&'a [f32], &'a mut [f32]),
}

impl<'a> SplitBuffer<'a> {
    /// Attempts to align both input and output slices to `F32x`.
    ///
    /// If the relative alignment of input and output differs, `body` will be empty,
    /// and the entire range will be returned in `head` for scalar processing.
    pub fn new(input: &'a [f32], output: &'a mut [f32]) -> Self {
        // Ensure lengths match to avoid boundary checks later.
        assert_eq!(input.len(), output.len());

        // Safety: f32 and F32x are POD types. align_to ensures valid sub-slices.
        let (in_head, in_body, in_tail) = unsafe { input.align_to::<F32x>() };

        // Calculate the expected head length for output without mutably borrowing it yet.
        let out_ptr = output.as_ptr();
        let align_req = core::mem::align_of::<F32x>();
        let out_offset = out_ptr.align_offset(align_req);
        let out_head_len = core::cmp::min(out_offset, output.len());

        // Check if the alignment boundaries match for both buffers.
        // We can only process in blocks if the scalar heads and tails are of equal length,
        // ensuring the SIMD bodies line up 1:1.
        if in_head.len() == out_head_len {
            let (out_head, out_body, out_tail) = unsafe { output.align_to_mut::<F32x>() };

            Self {
                head: (in_head, out_head),
                body: (in_body, out_body),
                tail: (in_tail, out_tail),
            }
        } else {
            // Fallback: If alignments mismatch, process everything as scalars.
            Self {
                head: (input, output),
                body: (&[], &mut []),
                tail: (&[], &mut []),
            }
        }
    }
}