use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};

use crate::time::{Duration, IngestionTime, Instant, Nanos};

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, Default)]
pub struct InternalMessage<T> {
    ingestion_t: IngestionTime,
    data: T,
}

impl<T> InternalMessage<T> {
    #[inline]
    pub fn new(ingestion_t: IngestionTime, data: T) -> Self {
        Self { ingestion_t, data }
    }

    #[inline]
    pub fn with_data<D>(&self, data: D) -> InternalMessage<D> {
        InternalMessage::new(self.ingestion_t, data)
    }

    #[inline]
    pub fn data(&self) -> &T {
        &self.data
    }

    #[inline]
    pub fn into_data(self) -> T {
        self.data
    }

    #[inline]
    pub fn map<R>(self, f: impl FnOnce(T) -> R) -> InternalMessage<R> {
        InternalMessage {
            ingestion_t: self.ingestion_t,
            data: f(self.data),
        }
    }

    #[inline]
    pub fn map_ref<R>(&self, f: impl FnOnce(&T) -> R) -> InternalMessage<R> {
        InternalMessage {
            ingestion_t: self.ingestion_t,
            data: f(&self.data),
        }
    }

    #[inline]
    pub fn unpack(self) -> (IngestionTime, T) {
        (self.ingestion_t, self.data)
    }

    /// This is only useful within the same socket as the original tsamp
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.ingestion_t.internal().elapsed()
    }

    /// These are real nanos since unix epoc
    #[inline]
    pub fn elapsed_nanos(&self) -> Nanos {
        self.ingestion_t.real().elapsed()
    }

    #[inline]
    pub fn ingestion_time(&self) -> IngestionTime {
        self.ingestion_t
    }
}

impl<T> From<InternalMessage<T>> for (IngestionTime, T) {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.unpack()
    }
}

impl<T> From<T> for InternalMessage<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self::new(IngestionTime::now(), value)
    }
}

impl<T> Deref for InternalMessage<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> DerefMut for InternalMessage<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T> From<&InternalMessage<T>> for IngestionTime {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t
    }
}

impl<T> AsRef<IngestionTime> for InternalMessage<T> {
    #[inline]
    fn as_ref(&self) -> &IngestionTime {
        &self.ingestion_t
    }
}

impl<T> From<&InternalMessage<T>> for Instant {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<&InternalMessage<T>> for Nanos {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<InternalMessage<T>> for Instant {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<InternalMessage<T>> for Nanos {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}
