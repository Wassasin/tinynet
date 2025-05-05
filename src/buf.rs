use core::mem::MaybeUninit;

/// A byte buffer that can be filled from the internal backing storage.
pub struct Buf<const N: usize> {
    buffer: [MaybeUninit<u8>; N],
    len: usize,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OverflowError;

impl<const N: usize> Buf<N> {
    pub const fn new() -> Self {
        Self {
            len: 0,
            buffer: [MaybeUninit::uninit(); N],
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        // NOTE(unsafe) avoid bound checks in the slicing operation
        // &buffer[..self.len]
        unsafe { core::slice::from_raw_parts(self.buffer.as_ptr() as *const u8, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // NOTE(unsafe) avoid bound checks in the slicing operation
        // &mut buffer[..self.len]
        unsafe { core::slice::from_raw_parts_mut(self.buffer.as_mut_ptr() as *mut u8, self.len) }
    }

    pub fn free_mut_slice(&mut self) -> &mut [u8] {
        // NOTE(unsafe) avoid bound checks in the slicing operation
        // &mut buffer[..self.len]
        unsafe {
            core::slice::from_raw_parts_mut(
                (self.buffer.as_mut_ptr() as *mut u8).offset(self.len as isize),
                N - self.len,
            )
        }
    }

    /// Mark `count` bytes as being filled.
    pub fn mark_valid(&mut self, count: usize) -> Result<(), OverflowError> {
        if count + self.len > N {
            return Err(OverflowError);
        }

        self.len += count;
        Ok(())
    }

    /// Remove front `count` bytes and moves the rest of the buffer forward.
    pub fn truncate_front(&mut self, count: usize) {
        self.buffer.copy_within(count.., 0);
        self.len = self.len.saturating_sub(count);
    }

    pub fn front(&self) -> Option<u8> {
        if self.len >= 1 {
            Some(unsafe { self.buffer[0].assume_init() })
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len == N
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}
