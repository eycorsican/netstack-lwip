use std::marker::PhantomPinned;
use std::ptr::null_mut;
use std::{net::SocketAddr, os::raw, pin::Pin};

use futures::stream::Stream;
use futures::task::{Context, Poll};
use log::*;
use tokio::sync::mpsc::{UnboundedSender, UnboundedReceiver, unbounded_channel};

use super::lwip::*;
use super::tcp_stream::TcpStream;
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
    let listener = unsafe { &mut *(arg as *mut TcpListener) };
    let stream = TcpStream::new(newpcb);
    let _ = listener.sender.send(stream);
    err_enum_t_ERR_OK as err_t
}

pub struct TcpListener {
    tpcb: usize,
    pub sender: UnboundedSender<Pin<Box<TcpStream>>>,
    pub receiver: UnboundedReceiver<Pin<Box<TcpStream>>>,
    _pin: PhantomPinned,
}

impl TcpListener {
    pub fn new() -> Result<Pin<Box<Self>>, Error> {
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
            let (sender, receiver) = unbounded_channel();
            let listener = Box::pin(TcpListener {
                tpcb: tpcb as usize,
                sender,
                receiver,
                _pin: PhantomPinned::default(),
            });
            let arg = &*listener as *const TcpListener as *mut raw::c_void;
            tcp_arg(tpcb, arg);
            tcp_accept(tpcb, Some(tcp_accept_cb));
            Ok(listener)
        }
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        unsafe {
            let _g = LWIP_MUTEX.lock();
            tcp_arg(self.tpcb as *mut tcp_pcb, null_mut());
            tcp_accept(self.tpcb as *mut tcp_pcb, None);
            tcp_close(self.tpcb as *mut tcp_pcb);
        }
    }
}

impl Stream for TcpListener {
    type Item = (Pin<Box<TcpStream>>, SocketAddr, SocketAddr);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let me = unsafe { self.get_unchecked_mut() };
        match me.receiver.poll_recv(cx) {
            Poll::Ready(Some(stream)) => {
                let local_addr = stream.local_addr().to_owned();
                let remote_addr = stream.remote_addr().to_owned();
                return Poll::Ready(Some((stream, local_addr, remote_addr)));
            },
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
