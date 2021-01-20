// Copyright 2020, Collabora Ltd.
// SPDX-License-Identifier: MIT

use libc::{c_char, c_void};
use std::ffi::CStr;
use std::mem;
use std::pin::Pin;

use crate::registry::ObjectType;

#[derive(Debug)]
pub struct Proxy(*mut pw_sys::pw_proxy);

// Wrapper around a proxy pointer
impl Proxy {
    pub(crate) fn new(proxy: *mut pw_sys::pw_proxy) -> Self {
        Proxy(proxy)
    }

    pub(crate) fn as_ptr(&self) -> *mut pw_sys::pw_proxy {
        self.0
    }

    pub fn add_listener_local(&self) -> ProxyListenerLocalBuilder {
        ProxyListenerLocalBuilder {
            proxy: &self,
            cbs: ListenerLocalCallbacks::default(),
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        unsafe {
            pw_sys::pw_proxy_destroy(self.0);
        }
    }
}

// Trait implemented by high level proxy wrappers
pub trait ProxyT {
    // Add Sized restriction on those methods so it can be used as a
    // trait object, see E0038
    fn type_() -> ObjectType
    where
        Self: Sized;

    fn new(proxy: Proxy) -> Self
    where
        Self: Sized;

    fn upcast(self) -> Proxy;
    fn upcast_ref(&self) -> &Proxy;
}

// Trait implemented by listener on high level proxy wrappers.
pub trait Listener {}

pub struct ProxyListener {
    // Need to stay allocated while the listener is registered
    #[allow(dead_code)]
    events: Pin<Box<pw_sys::pw_proxy_events>>,
    listener: Pin<Box<spa_sys::spa_hook>>,
    #[allow(dead_code)]
    data: Box<ListenerLocalCallbacks>,
}

impl<'a> Listener for ProxyListener {}

impl<'a> Drop for ProxyListener {
    fn drop(&mut self) {
        spa::hook::remove(*self.listener);
    }
}
#[derive(Default)]
struct ListenerLocalCallbacks {
    destroy: Option<Box<dyn Fn()>>,
    bound: Option<Box<dyn Fn(u32)>>,
    removed: Option<Box<dyn Fn()>>,
    done: Option<Box<dyn Fn(i32)>>,
    #[allow(clippy::type_complexity)]
    error: Option<Box<dyn Fn(i32, i32, &str)>>, // TODO: return a proper Error enum?
}

pub struct ProxyListenerLocalBuilder<'a> {
    proxy: &'a Proxy,
    cbs: ListenerLocalCallbacks,
}

impl<'a> ProxyListenerLocalBuilder<'a> {
    #[must_use]
    pub fn destroy<F>(mut self, destroy: F) -> Self
    where
        F: Fn() + 'static,
    {
        self.cbs.destroy = Some(Box::new(destroy));
        self
    }

    #[must_use]
    pub fn bound<F>(mut self, bound: F) -> Self
    where
        F: Fn(u32) + 'static,
    {
        self.cbs.bound = Some(Box::new(bound));
        self
    }

    #[must_use]
    pub fn removed<F>(mut self, removed: F) -> Self
    where
        F: Fn() + 'static,
    {
        self.cbs.removed = Some(Box::new(removed));
        self
    }

    #[must_use]
    pub fn done<F>(mut self, done: F) -> Self
    where
        F: Fn(i32) + 'static,
    {
        self.cbs.done = Some(Box::new(done));
        self
    }

    #[must_use]
    pub fn error<F>(mut self, error: F) -> Self
    where
        F: Fn(i32, i32, &str) + 'static,
    {
        self.cbs.error = Some(Box::new(error));
        self
    }

    #[must_use]
    pub fn register(self) -> ProxyListener {
        unsafe extern "C" fn proxy_destroy(data: *mut c_void) {
            let callbacks = (data as *mut ListenerLocalCallbacks).as_ref().unwrap();
            callbacks.destroy.as_ref().unwrap()();
        }

        unsafe extern "C" fn proxy_bound(data: *mut c_void, global_id: u32) {
            let callbacks = (data as *mut ListenerLocalCallbacks).as_ref().unwrap();
            callbacks.bound.as_ref().unwrap()(global_id);
        }

        unsafe extern "C" fn proxy_removed(data: *mut c_void) {
            let callbacks = (data as *mut ListenerLocalCallbacks).as_ref().unwrap();
            callbacks.removed.as_ref().unwrap()();
        }

        unsafe extern "C" fn proxy_done(data: *mut c_void, seq: i32) {
            let callbacks = (data as *mut ListenerLocalCallbacks).as_ref().unwrap();
            callbacks.done.as_ref().unwrap()(seq);
        }

        unsafe extern "C" fn proxy_error(
            data: *mut c_void,
            seq: i32,
            res: i32,
            message: *const c_char,
        ) {
            let callbacks = (data as *mut ListenerLocalCallbacks).as_ref().unwrap();
            let message = CStr::from_ptr(message).to_str().unwrap();
            callbacks.error.as_ref().unwrap()(seq, res, message);
        }

        let e = unsafe {
            let mut e: Pin<Box<pw_sys::pw_proxy_events>> = Box::pin(mem::zeroed());
            e.version = pw_sys::PW_VERSION_PROXY_EVENTS;

            if self.cbs.destroy.is_some() {
                e.destroy = Some(proxy_destroy);
            }

            if self.cbs.bound.is_some() {
                e.bound = Some(proxy_bound);
            }

            if self.cbs.removed.is_some() {
                e.removed = Some(proxy_removed);
            }

            if self.cbs.done.is_some() {
                e.done = Some(proxy_done);
            }

            if self.cbs.error.is_some() {
                e.error = Some(proxy_error);
            }

            e
        };

        let (listener, data) = unsafe {
            let proxy = &self.proxy.as_ptr();

            let data = Box::into_raw(Box::new(self.cbs));
            let mut listener: Pin<Box<spa_sys::spa_hook>> = Box::pin(mem::zeroed());
            let listener_ptr: *mut spa_sys::spa_hook = listener.as_mut().get_unchecked_mut();
            let funcs: *const pw_sys::pw_proxy_events = e.as_ref().get_ref();

            pw_sys::pw_proxy_add_listener(
                proxy.cast(),
                listener_ptr.cast(),
                funcs.cast(),
                data as *mut _,
            );

            (listener, Box::from_raw(data))
        };

        ProxyListener {
            events: e,
            listener,
            data,
        }
    }
}