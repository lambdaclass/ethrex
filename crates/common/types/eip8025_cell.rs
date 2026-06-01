/// `OnceCell` replacement for zkVM guest gated on `eip-8025` feature.
///
/// `once_cell::sync::OnceCell` atomics are pure overhead in zkVM guest.
/// This struct copies the methods from `once_cell::unsync::OnceCell` and uses unsafe
/// to get around the Sync requirement.
///
/// This code is only sound because the guest is guaranteed to be single-threaded.
#[cfg(all(feature = "eip-8025", not(target_arch = "riscv64")))]
compile_error!(
    "Feature `eip-8025` uses a single-threaded OnceCell. This is unsafe and should be used only in guest program running on riscv64im_zicclsm-unknown-none-elf."
);
pub struct OnceCell<T>(core::cell::UnsafeCell<Option<T>>);

unsafe impl<T: Sync> Sync for OnceCell<T> {}

impl<T> OnceCell<T> {
    #[inline]
    pub fn new() -> Self {
        Self(core::cell::UnsafeCell::new(None))
    }

    #[inline]
    pub fn get(&self) -> Option<&T> {
        unsafe { &*self.0.get() }.as_ref()
    }

    #[inline]
    pub fn get_or_init(&self, f: impl FnOnce() -> T) -> &T {
        match self.get_or_try_init(|| Ok::<T, core::convert::Infallible>(f())) {
            Ok(val) => val,
            Err(e) => match e {},
        }
    }

    #[inline]
    pub fn get_or_try_init<E>(&self, f: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
        if let Some(val) = self.get() {
            return Ok(val);
        }
        self.try_init(f)
    }

    #[inline]
    pub fn set(&self, value: T) -> Result<(), T> {
        match self.try_insert(value) {
            Ok(_) => Ok(()),
            Err((_, value)) => Err(value),
        }
    }

    #[inline]
    pub fn try_insert(&self, value: T) -> Result<&T, (&T, T)> {
        if let Some(old) = self.get() {
            return Err((old, value));
        }
        let slot = unsafe { &mut *self.0.get() };
        Ok(slot.insert(value))
    }

    #[inline]
    fn try_init<E>(&self, f: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
        let val = f()?;
        let slot = unsafe { &mut *self.0.get() };
        debug_assert!(slot.is_none());
        Ok(slot.insert(val))
    }
}

impl<T: PartialEq> PartialEq for OnceCell<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl<T> Default for OnceCell<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Eq> Eq for OnceCell<T> {}

impl<T: Clone> Clone for OnceCell<T> {
    #[inline]
    fn clone(&self) -> OnceCell<T> {
        match self.get() {
            Some(value) => OnceCell::from(value.clone()),
            None => OnceCell::new(),
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for OnceCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_tuple("OnceCell");
        match self.get() {
            Some(v) => d.field(v),
            None => d.field(&format_args!("<uninit>")),
        };
        d.finish()
    }
}

impl<T> From<T> for OnceCell<T> {
    #[inline]
    fn from(value: T) -> Self {
        OnceCell(core::cell::UnsafeCell::new(Some(value)))
    }
}
