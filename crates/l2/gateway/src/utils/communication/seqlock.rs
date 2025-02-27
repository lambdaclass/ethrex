use std::{
    cell::UnsafeCell,
    fmt,
    mem::MaybeUninit,
    sync::atomic::{compiler_fence, AtomicU32, Ordering},
};

use super::ReadError;
//TODO: Make the types more rust like. I.e. on copy types -> copy on write/read, clone types ->
// copy std::mem::forget till read etc
/// A sequential lock
#[repr(C, align(64))]
pub struct Seqlock<T> {
    pub data: UnsafeCell<T>, // don't change this order or rust does padding till 8bytes
    pub version: AtomicU32,
}
unsafe impl<T: Send> Send for Seqlock<T> {}
unsafe impl<T: Send> Sync for Seqlock<T> {}

// TODO: Try 32 bit version
impl<T: Clone> Seqlock<T> {
    /// Creates a new SeqLock with the given initial value.
    #[inline]
    pub const fn new(val: T) -> Seqlock<T> {
        Seqlock {
            version: AtomicU32::new(0),
            data: UnsafeCell::new(val),
        }
    }

    pub fn version(&self) -> u32 {
        self.version.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    fn set_version(&self, v: u32) {
        self.version.store(v, Ordering::Relaxed)
    }

    #[inline(never)]
    pub fn read_with_version(
        &self,
        result: &mut T,
        expected_version: u32,
    ) -> Result<(), ReadError> {
        let v1 = self.version.load(Ordering::Acquire);
        if v1 < expected_version {
            return Err(ReadError::Empty);
        }

        compiler_fence(Ordering::AcqRel);
        *result = unsafe { (*self.data.get()).clone() };
        compiler_fence(Ordering::AcqRel);
        let v2 = self.version.load(Ordering::Acquire);
        if v2 == expected_version {
            Ok(())
        } else {
            Err(ReadError::SpedPast)
        }
    }

    #[inline(never)]
    pub fn read_with_version_clone(&self, expected_version: u32) -> Result<T, ReadError> {
        let v1 = self.version.load(Ordering::Acquire);
        if v1 < expected_version {
            return Err(ReadError::Empty);
        }

        compiler_fence(Ordering::AcqRel);
        let result = unsafe { MaybeUninit::new((*self.data.get()).clone()) };
        compiler_fence(Ordering::AcqRel);
        let v2 = self.version.load(Ordering::Acquire);
        if v2 == expected_version {
            Ok(unsafe { result.assume_init() })
        } else {
            Err(ReadError::SpedPast)
        }
    }

    #[inline(never)]
    pub fn read(&self, result: &mut T) {
        loop {
            let v1 = self.version.load(Ordering::Acquire);
            compiler_fence(Ordering::AcqRel);
            unsafe {
                *result = (*self.data.get()).clone();
            }
            compiler_fence(Ordering::AcqRel);
            let v2 = self.version.load(Ordering::Acquire);
            if v1 == v2 && v1 & 1 == 0 {
                return;
            }
            #[cfg(target_arch = "x86_64")]
            unsafe {
                std::arch::x86_64::_mm_pause()
            };
        }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub fn view_unsafe(&self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }

    #[inline(never)]
    pub fn write(&self, val: &T) {
        // Increment the sequence number. At this point, the number will be odd,
        // which will force readers to spin until we finish writing.
        let v = self.version.fetch_add(1, Ordering::Release);
        compiler_fence(Ordering::AcqRel);
        if v != 0 {
            unsafe { *self.data.get() = val.clone() };
        } else {
            // first time we write, shouldn't drop this value
            unsafe { std::ptr::write(self.data.get(), val.clone()) };
        }
        // Make sure any writes to the data happen after incrementing the
        // sequence number. What we ideally want is a store(Acquire), but the
        // Acquire ordering is not available on stores.
        compiler_fence(Ordering::AcqRel);
        // unsafe {asm!("sti");}
        self.version.store(v.wrapping_add(2), Ordering::Release);
    }

    #[inline(never)]
    pub fn _write_unpoison<F>(&self, f: F)
    where
        F: FnOnce(),
    {
        let v = self.version.load(Ordering::Relaxed);
        self.version
            .store(v.wrapping_add(v.wrapping_sub(1) & 1), Ordering::Release);
        // Make sure any writes to the data happen after incrementing the
        // sequence number. What we ideally want is a store(Acquire), but the
        // Acquire ordering is not available on stores.
        compiler_fence(Ordering::AcqRel);
        f();
        compiler_fence(Ordering::AcqRel);
        self.version.store(v.wrapping_add(1), Ordering::Relaxed);
    }

    #[inline]
    pub fn write_unpoison(&self, val: &T) {
        self._write_unpoison(|| {
            let t = self.data.get() as *mut u8;
            unsafe { t.copy_from(val as *const _ as *const u8, std::mem::size_of::<T>()) };
        });
    }

    #[inline(always)]
    #[allow(named_asm_labels)]
    fn _write_multi<F>(&self, f: F)
    where
        F: FnOnce(),
    {
        // Increment the sequence number. At this point, the number will be odd,
        // which will force readers to spin until we finish writing.
        let mut v = self.version.fetch_or(1, Ordering::AcqRel);
        while v & 1 == 1 {
            v = self.version.fetch_or(1, Ordering::AcqRel);
        }
        // Make sure any writes to the data happen after incrementing the
        // sequence number. What we ideally want is a store(Acquire), but the
        // Acquire ordering is not available on stores.
        f();
        compiler_fence(Ordering::AcqRel);
        self.version.store(v.wrapping_add(2), Ordering::Release);
    }

    #[inline(never)]
    pub fn write_multi(&self, val: &T) {
        self._write_multi(|| {
            unsafe { self.data.get().copy_from(val as *const T, 1) };
        });
    }
}

impl<T: Clone + Default> Default for Seqlock<T> {
    #[inline]
    fn default() -> Seqlock<T> {
        Seqlock::new(Default::default())
    }
}

impl<T: Clone + fmt::Debug> fmt::Debug for Seqlock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SeqLock {{ data: {:?} }}", unsafe {
            (*self.data.get()).clone()
        })
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };

    use super::*;

    #[test]
    fn lock_size() {
        assert_eq!(std::mem::size_of::<Seqlock<[u8; 48]>>(), 64);
        assert_eq!(std::mem::size_of::<Seqlock<[u8; 61]>>(), 128)
    }

    fn consumer_loop<const N: usize>(lock: &Seqlock<[usize; N]>, done: &AtomicBool) {
        let mut msg = [0usize; N];
        while !done.load(Ordering::Relaxed) {
            lock.read(&mut msg);
            let first = msg[0];
            for i in msg {
                assert_eq!(first, i);
            }
        }
    }

    fn producer_loop<const N: usize>(lock: &Seqlock<[usize; N]>, done: &AtomicBool, multi: bool) {
        let curt = Instant::now();
        let mut count = 0;
        let mut msg = [0usize; N];
        while curt.elapsed() < Duration::from_secs(1) {
            msg.fill(count);
            if multi {
                lock.write_multi(&msg);
            } else {
                lock.write(&msg);
            }
            count = count.wrapping_add(1);
        }
        done.store(true, Ordering::Relaxed);
    }

    fn read_test<const N: usize>() {
        let lock = Seqlock::new([0usize; N]);
        let done = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                consumer_loop(&lock, &done);
            });
            s.spawn(|| {
                producer_loop(&lock, &done, false);
            });
        });
    }

    fn read_test_multi<const N: usize>() {
        let lock = Seqlock::new([0usize; N]);
        let done = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                consumer_loop(&lock, &done);
            });
            s.spawn(|| {
                producer_loop(&lock, &done, true);
            });
            s.spawn(|| {
                producer_loop(&lock, &done, true);
            });
        });
    }

    #[test]
    fn read_16() {
        read_test::<16>()
    }
    #[test]
    fn read_32() {
        read_test::<32>()
    }
    #[test]
    fn read_64() {
        read_test::<64>()
    }
    #[test]
    fn read_128() {
        read_test::<128>()
    }
    #[test]
    fn read_large() {
        read_test::<1024>()
    }

    #[test]
    fn read_16_multi() {
        read_test_multi::<16>()
    }
    #[test]
    fn read_32_multi() {
        read_test_multi::<32>()
    }
    #[test]
    fn read_64_multi() {
        read_test_multi::<64>()
    }
    #[test]
    fn read_128_multi() {
        read_test_multi::<128>()
    }
    #[test]
    fn read_large_multi() {
        read_test_multi::<1024>()
    }

    #[test]
    fn write_unpoison() {
        let lock = Seqlock::default();
        lock.set_version(1);
        lock.write_unpoison(&1);
        assert_eq!(lock.version(), 2);
    }
}
