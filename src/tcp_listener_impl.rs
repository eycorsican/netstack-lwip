use std::ptr::null_mut;
use std::{collections::VecDeque, net::SocketAddr, os::raw, pin::Pin};

use futures::stream::Stream;
use futures::task::{Context, Poll, Waker};
use log::*;

use super::lwip::*;
use super::tcp_stream::TcpStream;
use super::tcp_stream_impl::TcpStreamImpl;
use super::LWIP_MUTEX;
use crate::Error;

#[allow(unused_variables)]
pub extern "C" fn tcp_accept_cb(arg: *mut raw::c_void, newpcb: *mut tcp_pcb, err: err_t) -> err_t {
    if arg.is_null() {
        warn!("tcp listener has been closed");
        return err_enum_t_ERR_CONN as err_t;
    }
    if newpcb.is_null() {
        warn!("tcp full");
        return err_enum_t_ERR_OK as err_t;
    }
    if err != err_enum_t_ERR_OK as err_t {
        warn!("accept tcp failed: {}", err);
        // Not sure what to do if there was an error, just ignore it.
        return err_enum_t_ERR_OK as err_t;
    }
    let listener = unsafe { &mut *(arg as *mut TcpListenerImpl) };
    let stream = TcpStreamImpl::new(newpcb);
    listener.queue.push_back(stream);
    if let Some(waker) = listener.waker.as_ref() {
        waker.wake_by_ref();
    }
    err_enum_t_ERR_OK as err_t
}

pub struct TcpListenerImpl {
    pub tpcb: usize,
    pub waker: Option<Waker>,
    pub queue: VecDeque<Box<TcpStreamImpl>>,
}

impl TcpListenerImpl {
    pub fn new() -> Result<Box<Self>, Error> {
        unsafe {
            let _g = LWIP_MUTEX.lock();
            let mut tpcb = tcp_new();
            let err = tcp_bind(tpcb, &ip_addr_any_type, 0);
            if err != err_enum_t_ERR_OK as err_t {
                error!("bind TCP failed: {}", err);
                return Err(Error::LwIP(err));
            }
            let mut reason: err_t = 0;
            tpcb = tcp_listen_with_backlog_and_err(
                tpcb,
                TCP_DEFAULT_LISTEN_BACKLOG as u8,
                &mut reason,
            );
            if tpcb.is_null() {
                error!("listen TCP failed: {}", reason);
                return Err(Error::LwIP(reason));
            }
            let listener = Box::new(TcpListenerImpl {
                tpcb: tpcb as usize,
                waker: None,
                queue: VecDeque::new(),
            });
            let arg = &*listener as *const TcpListenerImpl as *mut raw::c_void;
            tcp_arg(tpcb, arg);
            tcp_accept(tpcb, Some(tcp_accept_cb));
            Ok(listener)
        }
    }
}

impl Drop for TcpListenerImpl {
    fn drop(&mut self) {
        unsafe {
            let _g = LWIP_MUTEX.lock();
            tcp_arg(self.tpcb as *mut tcp_pcb, null_mut());
            tcp_accept(self.tpcb as *mut tcp_pcb, None);
            tcp_close(self.tpcb as *mut tcp_pcb);
        }
    }
}

impl Stream for TcpListenerImpl {
    type Item = (TcpStream, SocketAddr, SocketAddr);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if let Some(stream) = self.queue.pop_front() {
            let local_addr = stream.local_addr().to_owned();
            let remote_addr = stream.remote_addr().to_owned();
            return Poll::Ready(Some((TcpStream::new(stream), local_addr, remote_addr)));
        } else {
            self.waker.replace(cx.waker().clone());
            Poll::Pending
        }
    }
}
