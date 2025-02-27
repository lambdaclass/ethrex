use std::{
    alloc::Layout,
    borrow::Borrow,
    mem::size_of,
    ops::Deref,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

use shared_memory::ShmemConf;
use tracing::error;

use super::{seqlock::Seqlock, Error, ReadError};

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum QueueType {
    Unknown,
    MPMC,
    SPMC,
}

#[derive(Debug)]
#[repr(C, align(64))]
struct QueueHeader {
    queue_type: QueueType, // 1
    is_initialized: u8,    // 2
    _pad1: [u8; 6],        // 8
    elsize: usize,         // 16
    mask: usize,           // 24
    count: AtomicUsize,    // 32
}
#[allow(dead_code)]
impl QueueHeader {
    /// in bytes
    pub fn size_of(&self) -> usize {
        (self.mask + 1) * self.elsize
    }

    pub fn len(&self) -> usize {
        self.mask + 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn from_ptr(ptr: *mut u8) -> &'static mut Self {
        unsafe { &mut *(ptr as *mut Self) }
    }

    pub fn is_initialized(&self) -> bool {
        self.is_initialized == 1
    }

    pub fn elsize(&self) -> usize {
        self.elsize
    }

    pub fn open_shared<S: AsRef<Path>>(path: S) -> &'static mut Self {
        let shmem = ShmemConf::new().flink(path.as_ref()).open().unwrap();
        let ptr = shmem.as_ptr();
        std::mem::forget(shmem);
        Self::from_ptr(ptr)
    }
}

#[repr(C, align(64))]
pub struct InnerQueue<T> {
    header: QueueHeader,
    buffer: [Seqlock<T>],
}

impl<T> InnerQueue<T> {
    /// Allocs (unshared) memory and initializes a new queue from it.
    ///     QueueType::MPMC = multi producer multi consumer
    ///     QueueType::SPMC = single producer multi consumer
    fn new(len: usize, queue_type: QueueType) -> Result<*const Self, Error> {
        let real_len = len.next_power_of_two();
        let size = size_of::<QueueHeader>() + real_len * size_of::<Seqlock<T>>();

        unsafe {
            let ptr = std::alloc::alloc_zeroed(Layout::array::<u8>(size).unwrap().align_to(64).unwrap().pad_to_align());
            // Why real len you may ask. The size of the fat pointer ONLY includes the length of the
            // unsized part of the struct i.e. the buffer.
            Self::from_uninitialized_ptr(ptr, real_len, queue_type)
        }
    }

    fn from_uninitialized_ptr(ptr: *mut u8, len: usize, queue_type: QueueType) -> Result<*const Self, Error> {
        if !len.is_power_of_two() {
            return Err(Error::LengthNotPowerOfTwo);
        }
        unsafe {
            let q = std::ptr::slice_from_raw_parts_mut(ptr, len) as *mut Self;
            let elsize = size_of::<Seqlock<T>>();
            if !len.is_power_of_two() {
                return Err(Error::LengthNotPowerOfTwo);
            }

            let mask = len - 1;

            (*q).header.queue_type = queue_type;
            (*q).header.mask = mask;
            (*q).header.elsize = elsize;
            (*q).header.is_initialized = true as u8;
            (*q).header.count = AtomicUsize::new(0);
            Ok(q)
        }
    }

    fn create_or_open_shared<P: AsRef<Path>>(shmem_file: P, len: usize, typ: QueueType) -> Result<*const Self, Error> {
        use shared_memory::{ShmemConf, ShmemError};
        let _ = std::fs::create_dir_all(shmem_file.as_ref().parent().unwrap());
        match ShmemConf::new().size(Self::size_of(len)).flink(&shmem_file).create() {
            Ok(shmem) => {
                let ptr = shmem.as_ptr();
                std::mem::forget(shmem);
                Ok(Self::from_uninitialized_ptr(ptr, len, typ)?)
            }
            Err(ShmemError::LinkExists) => {
                let v = Self::open_shared(&shmem_file)?;
                if unsafe { (*v).header.len() } < len {
                    Err(Error::TooSmall)
                } else {
                    Ok(v)
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    fn open_shared<S: AsRef<Path>>(shmem_file: S) -> Result<*const Self, Error> {
        if !shmem_file.as_ref().exists() {
            return Err(Error::NonExistingFile);
        }
        let mut tries = 0;
        let mut header = QueueHeader::open_shared(shmem_file.as_ref());
        while !header.is_initialized() {
            std::thread::sleep(std::time::Duration::from_millis(1));
            header = QueueHeader::open_shared(shmem_file.as_ref());
            if tries == 10 {
                return Err(Error::UnInitialized);
            }
            tries += 1;
        }
        Self::from_initialized_ptr(header)
    }

    fn from_initialized_ptr(ptr: *mut QueueHeader) -> Result<*const Self, Error> {
        unsafe {
            let len = (*ptr).mask + 1;
            if !len.is_power_of_two() {
                return Err(Error::LengthNotPowerOfTwo);
            }
            if (*ptr).is_initialized != true as u8 {
                return Err(Error::UnInitialized);
            }
            Ok(std::ptr::slice_from_raw_parts_mut(ptr as *mut Seqlock<T>, len) as *const Self)
        }
    }

    const fn size_of(len: usize) -> usize {
        size_of::<QueueHeader>() + len.next_power_of_two() * size_of::<Seqlock<T>>()
    }
}

impl<T: Clone> InnerQueue<T> {
    // Note: Calling this from anywhere that's not a producer -> false sharing
    fn count(&self) -> usize {
        self.header.count.load(Ordering::Relaxed)
    }

    fn next_count(&self) -> usize {
        match self.header.queue_type {
            QueueType::Unknown => panic!("Unknown queue"),
            QueueType::MPMC => self.header.count.fetch_add(1, Ordering::AcqRel),
            QueueType::SPMC => {
                let c = self.header.count.load(Ordering::Relaxed);
                self.header.count.store(c.wrapping_add(1), Ordering::Relaxed);
                c
            }
        }
    }

    fn load(&self, pos: usize) -> &Seqlock<T> {
        unsafe { self.buffer.get_unchecked(pos) }
    }

    fn cur_pos(&self) -> usize {
        self.count() & self.header.mask
    }

    fn version(&self) -> u32 {
        (((self.count() / (self.header.mask + 1)) << 1) + 2) as u32
    }

    #[allow(dead_code)]
    fn version_of(&self, pos: usize) -> u32 {
        self.load(pos).version()
    }

    // returns the current count
    fn produce(&self, item: &T) -> usize {
        let p = self.next_count();
        let lock = self.load(p & self.header.mask);
        lock.write(item);
        p
    }

    fn consume(&self, el: &mut T, ri: usize, ri_ver: u32) -> Result<(), ReadError> {
        self.load(ri).read_with_version(el, ri_ver)
    }

    fn consume_clone(&self, ri: usize, ri_ver: u32) -> Result<T, ReadError> {
        self.load(ri).read_with_version_clone(ri_ver)
    }

    #[allow(dead_code)]
    fn read(&self, el: &mut T, ri: usize) {
        self.load(ri).read(el)
    }

    fn len(&self) -> usize {
        self.header.mask + 1
    }

    fn produce_first(&self, item: &T) -> usize {
        match self.header.queue_type {
            QueueType::Unknown => panic!("Unknown queue"),
            QueueType::MPMC => self.produce(item),
            QueueType::SPMC => {
                let m = self.header.mask;
                let c = self.count();
                let p = c & m;
                let lock = self.load(p);
                if lock.version() & 1 == 1 {
                    lock.write_unpoison(item);
                    p
                } else {
                    self.produce(item)
                }
            }
        }
    }
}

unsafe impl<T> Send for InnerQueue<T> {}
unsafe impl<T> Sync for InnerQueue<T> {}

impl<T: std::fmt::Debug> std::fmt::Debug for InnerQueue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Queue:\nHeader:\n{:?}", self.header)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Queue<T> {
    inner: *const InnerQueue<T>,
}

impl<T> Queue<T> {
    pub fn new(len: usize, queue_type: QueueType) -> Result<Self, Error> {
        InnerQueue::new(len, queue_type).map(|inner| Self { inner })
    }

    pub fn create_or_open_shared<P: AsRef<Path>>(
        shmem_file: P,
        len: usize,
        queue_type: QueueType,
    ) -> Result<Self, Error> {
        InnerQueue::create_or_open_shared(shmem_file, len, queue_type).map(|inner| Self { inner })
    }

    pub fn open_shared<P: AsRef<Path>>(shmem_file: P) -> Result<Self, Error> {
        InnerQueue::open_shared(shmem_file).map(|inner| Self { inner })
    }
}

unsafe impl<T> Send for Queue<T> {}
unsafe impl<T> Sync for Queue<T> {}

impl<T> Borrow<InnerQueue<T>> for Queue<T> {
    fn borrow(&self) -> &InnerQueue<T> {
        unsafe { &*self.inner }
    }
}
impl<T> Deref for Queue<T> {
    type Target = InnerQueue<T>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner }
    }
}
impl<T> AsRef<InnerQueue<T>> for Queue<T> {
    fn as_ref(&self) -> &InnerQueue<T> {
        unsafe { &*self.inner }
    }
}

/// Simply exists for the automatic produce_first
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Producer<T> {
    pub produced_first: u8, // 1
    pub queue: Queue<T>,
}

impl<T: Clone> From<Queue<T>> for Producer<T> {
    fn from(queue: Queue<T>) -> Self {
        Self { produced_first: 0, queue }
    }
}

impl<T: Clone> Producer<T> {
    pub fn produce(&mut self, msg: &T) -> usize {
        if self.produced_first == 0 {
            self.produced_first = 1;
            self.queue.produce_first(msg)
        } else {
            self.queue.produce(msg)
        }
    }

    pub fn produce_without_first(&self, msg: &T) -> usize {
        self.queue.produce(msg)
    }
}

impl<T> AsMut<Producer<T>> for Producer<T> {
    fn as_mut(&mut self) -> &mut Producer<T> {
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ConsumerBare<T> {
    pos: usize,            // 8
    mask: usize,           // 16
    expected_version: u32, // 20
    is_running: u8,        // 21
    _pad: [u8; 11],        // 32
    queue: Queue<T>,       // 48 fat ptr: (usize, pointer)
}

impl<T: Clone> ConsumerBare<T> {
    #[inline]
    pub fn recover_after_error(&mut self) {
        self.expected_version += 2;
    }

    #[inline]
    fn update_pos(&mut self) {
        self.pos = (self.pos + 1) & self.mask;
        self.expected_version = self.expected_version.wrapping_add(2 * (self.pos == 0) as u32);
    }

    /// Nonblocking consume returning either Ok(()) or a ReadError
    #[inline]
    pub fn try_consume(&mut self, el: &mut T) -> Result<(), ReadError> {
        self.queue.consume(el, self.pos, self.expected_version)?;
        self.update_pos();
        Ok(())
    }

    #[inline]
    pub fn try_consume_clone(&mut self) -> Result<T, ReadError> {
        let t = self.queue.consume_clone(self.pos, self.expected_version)?;
        self.update_pos();
        Ok(t)
    }

    /// Blocking consume
    #[inline]
    pub fn blocking_consume(&mut self, el: &mut T) {
        loop {
            match self.try_consume(el) {
                Ok(_) => {
                    return;
                }
                Err(ReadError::Empty) => {
                    #[cfg(target_arch = "x86_64")]
                    unsafe {
                        std::arch::x86_64::_mm_pause()
                    };
                    continue;
                }
                Err(ReadError::SpedPast) => {
                    self.recover_after_error();
                }
            }
        }
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn init_header(consumer_ptr: *mut ConsumerBare<T>, queue: Queue<T>) {
        unsafe {
            (*consumer_ptr).pos = queue.cur_pos();
            (*consumer_ptr).expected_version = queue.version();
            (*consumer_ptr).mask = queue.header.mask;
            (*consumer_ptr).queue = queue
        }
    }

    #[inline]
    pub fn tot_published(&self) -> usize {
        (*self.queue).count()
    }
}

impl<T> AsMut<ConsumerBare<T>> for ConsumerBare<T> {
    fn as_mut(&mut self) -> &mut ConsumerBare<T> {
        self
    }
}

impl<T: Clone> From<Queue<T>> for ConsumerBare<T> {
    fn from(queue: Queue<T>) -> Self {
        let pos = queue.cur_pos();
        let expected_version = queue.version();
        Self { pos, mask: queue.header.mask, _pad: [0; 11], expected_version, is_running: 1, queue }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Consumer<T: 'static> {
    consumer: ConsumerBare<T>,
    should_log: bool,
}

impl<T: 'static + Clone> Consumer<T> {
    #[inline]
    pub fn try_consume(&mut self) -> Option<T> {
        match self.consumer.try_consume_clone() {
            Ok(t) => Some(t),
            Err(ReadError::SpedPast) => {
                self.log_and_recover();
                self.try_consume()
            }
            Err(ReadError::Empty) => None,
        }
    }

    #[inline(never)]
    fn log_and_recover(&mut self) {
        if self.should_log {
            error!(
                "Consumer<{}> got sped past. Lost {} messages",
                std::any::type_name::<T>(),
                self.consumer.queue.len()
            );
        }
        self.consumer.recover_after_error();
    }

    #[inline]
    pub fn without_log(self) -> Self {
        Self { should_log: false, ..self }
    }

    #[inline]
    pub fn tot_published(&self) -> usize {
        self.consumer.tot_published()
    }
}

impl<T: Clone, Q: Into<ConsumerBare<T>>> From<Q> for Consumer<T> {
    fn from(queue: Q) -> Self {
        Self { consumer: queue.into(), should_log: true }
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod test {
    use super::*;

    #[test]
    fn headersize() {
        assert_eq!(64, std::mem::size_of::<QueueHeader>());
        assert_eq!(48, std::mem::size_of::<ConsumerBare<[u8; 60]>>())
    }

    #[test]
    fn basic() {
        for typ in [QueueType::SPMC, QueueType::MPMC] {
            let q = Queue::new(16, typ).unwrap();
            let mut p = Producer::from(q);
            let mut c = ConsumerBare::from(q);
            p.produce(&1);
            let mut m = 0;

            assert_eq!(c.try_consume(&mut m), Ok(()));
            assert_eq!(m, 1);
            assert!(matches!(c.try_consume(&mut m), Err(ReadError::Empty)));
            for i in 0..16 {
                p.produce(&i);
            }
            for i in 0..16 {
                c.try_consume(&mut m).unwrap();
                assert_eq!(m, i);
            }

            assert!(matches!(c.try_consume(&mut m), Err(ReadError::Empty)));

            for _ in 0..20 {
                p.produce(&1);
            }

            assert!(matches!(c.try_consume(&mut m), Err(ReadError::SpedPast)));
        }
    }

    fn multithread(n_writers: usize, n_readers: usize, tot_messages: usize) {
        let q = Queue::new(16, QueueType::MPMC).unwrap();

        let mut readhandles = Vec::new();
        for _ in 0..n_readers {
            let mut c1 = ConsumerBare::from(q);
            let cons = std::thread::spawn(move || {
                let mut n = 0;
                let mut c = 0;
                while n < tot_messages {
                    match c1.try_consume_clone() {
                        Ok(t) => {
                            n += 1;
                            c += t;
                        }
                        Err(ReadError::Empty) => {}
                        Err(ReadError::SpedPast) => panic!("sped past"),
                    }
                }
                assert_eq!(c, (0..tot_messages).sum::<usize>());
            });
            readhandles.push(cons)
        }
        let mut writehandles = Vec::new();
        for n in 0..n_writers {
            let mut p1 = Producer::from(q);
            let prod1 = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                let mut c = n;
                while c < tot_messages {
                    p1.produce(&c);
                    c += n_writers;
                    std::thread::sleep(std::time::Duration::from_micros(1));
                }
            });
            writehandles.push(prod1);
        }

        for h in readhandles {
            let _ = h.join();
        }
        for h in writehandles {
            let _ = h.join();
        }
    }

    #[test]
    fn multithread_1_2() {
        multithread(1, 2, 1000);
    }

    #[test]
    fn multithread_1_4() {
        multithread(1, 4, 1000);
    }

    #[test]
    fn multithread_2_4() {
        multithread(2, 4, 1000);
    }

    #[test]
    fn multithread_4_4() {
        multithread(4, 4, 1000);
    }

    #[test]
    fn multithread_8_8() {
        multithread(8, 8, 1000);
    }

    #[ignore = "Requires creating a file in /dev/shm"]
    #[test]
    fn basic_shared() {
        for typ in [QueueType::SPMC, QueueType::MPMC] {
            let path = std::path::Path::new("/dev/shm/blabla_test");
            let _ = std::fs::remove_file(path);
            let q = Queue::create_or_open_shared(path, 16, typ).unwrap();
            let mut p = Producer::from(q);
            let mut c = ConsumerBare::from(q);

            p.produce(&1);
            let mut m = 0;

            assert_eq!(c.try_consume(&mut m), Ok(()));
            assert_eq!(m, 1);
            assert!(matches!(c.try_consume(&mut m), Err(ReadError::Empty)));
            for i in 0..16 {
                p.produce(&i);
            }
            for i in 0..16 {
                c.try_consume(&mut m).unwrap();
                assert_eq!(m, i);
            }

            assert!(matches!(c.try_consume(&mut m), Err(ReadError::Empty)));

            for _ in 0..20 {
                p.produce(&1);
            }

            assert!(matches!(c.try_consume(&mut m), Err(ReadError::SpedPast)));
            let _ = std::fs::remove_file(path);
        }
    }
}
