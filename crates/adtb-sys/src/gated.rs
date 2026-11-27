//! Shared values whose every use runs as engine code.

use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::sync::{Arc, Weak};

use crate::panics::{Inside, inside};

/// A shared value reached only through [`Gated::enter`], which marks the calling thread as
/// running engine code for as long as the value is borrowed. The last handle's drop of the value
/// runs marked too.
pub struct Gated<T>(ManuallyDrop<Arc<T>>);

impl<T> Gated<T> {
    pub fn new(value: T) -> Gated<T> {
        Gated(ManuallyDrop::new(Arc::new(value)))
    }

    /// Returns the value, the calling thread marked as running engine code while it is borrowed.
    pub fn enter(&self) -> Entry<'_, T> {
        Entry {
            value: &self.0,
            _inside: inside(),
        }
    }

    pub fn downgrade(&self) -> GatedWeak<T> {
        GatedWeak(Arc::downgrade(&self.0))
    }

    /// Returns whether `self` and `other` share one value.
    pub fn ptr_eq(&self, other: &Gated<T>) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Clone for Gated<T> {
    fn clone(&self) -> Self {
        Gated(ManuallyDrop::new(Arc::clone(&self.0)))
    }
}

impl<T> Drop for Gated<T> {
    fn drop(&mut self) {
        let _inside = inside();
        // SAFETY: `self.0` is dropped once, here, and never used after.
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Gated<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Gated").field(&**self.0).finish()
    }
}

/// A weak handle on a [`Gated`] value, which gives no access to the value but through a
/// [`Gated`].
pub struct GatedWeak<T>(Weak<T>);

impl<T> GatedWeak<T> {
    /// Returns a handle that upgrades to nothing.
    pub fn new() -> GatedWeak<T> {
        GatedWeak(Weak::new())
    }

    /// Returns a handle on the value, `None` once every [`Gated`] handle on it has dropped.
    pub fn upgrade(&self) -> Option<Gated<T>> {
        self.0.upgrade().map(|v| Gated(ManuallyDrop::new(v)))
    }

    /// Returns whether `self` and `other` are handles on one value.
    pub fn ptr_eq(&self, other: &GatedWeak<T>) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }

    /// Returns whether `self` is a handle on the value of `gated`.
    pub fn is(&self, gated: &Gated<T>) -> bool {
        std::ptr::eq(self.0.as_ptr(), Arc::as_ptr(&gated.0))
    }
}

impl<T> Default for GatedWeak<T> {
    fn default() -> Self {
        GatedWeak::new()
    }
}

impl<T> Clone for GatedWeak<T> {
    fn clone(&self) -> Self {
        GatedWeak(Weak::clone(&self.0))
    }
}

impl<T> std::fmt::Debug for GatedWeak<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatedWeak").finish_non_exhaustive()
    }
}

/// A [`Gated`] value borrowed with the calling thread marked as running engine code.
#[derive(Debug)]
pub struct Entry<'g, T> {
    value: &'g T,
    _inside: Inside,
}

impl<'g, T> Entry<'g, T> {
    /// Returns the value for the whole borrow of its [`Gated`] handle, with the mark that keeps
    /// the calling thread marked.
    ///
    /// Precondition: the caller keeps the mark for as long as it uses the value.
    pub fn detach(self) -> (&'g T, Inside) {
        (self.value, self._inside)
    }
}

impl<T> Deref for Entry<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}
