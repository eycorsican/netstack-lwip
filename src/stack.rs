use std::marker::PhantomPinned;
use std::{io, os::raw, pin::Pin, sync::Once, time};

use futures::sink::Sink;
use futures::stream::Stream;
use futures::task::{Context, Poll, Waker};
use tokio::sync::mpsc::{channel, Receiver, Sender};

use super::lwip::*;
use super::output::{output_ip4, output_ip6, OUTPUT_CB_PTR};
use super::tcp_listener::TcpListener;
use super::udp::UdpSocket;
use super::LWIP_MUTEX;
use crate::Error;

static LWIP_INIT: Once = Once::new();

pub struct NetStack {
    waker: Option<Waker>,
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    sink_buf: Option<Vec<u8>>, // We're flushing per item, no need large buffer.
    _pin: PhantomPinned,
}

impl NetStack {
    pub fn new() -> Result<(Pin<Box<Self>>, Pin<Box<TcpListener>>, Pin<Box<UdpSocket>>), Error> {
        Ok((
            NetStack::_new(512),
            TcpListener::new()?,
            UdpSocket::new(64)?,
        ))
    }

    pub fn with_buffer_size(
        stack_buffer_size: usize,
        udp_buffer_size: usize,
    ) -> Result<(Pin<Box<Self>>, Pin<Box<TcpListener>>, Pin<Box<UdpSocket>>), Error> {
        Ok((
            NetStack::_new(stack_buffer_size),
            TcpListener::new()?,
            UdpSocket::new(udp_buffer_size)?,
        ))
    }

    fn _new(buffer_size: usize) -> Pin<Box<Self>> {
        LWIP_INIT.call_once(|| unsafe { lwip_init() });

        unsafe {
            (*netif_list).output = Some(output_ip4);
            (*netif_list).output_ip6 = Some(output_ip6);
            (*netif_list).mtu = 1500;
        }

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel(buffer_size);

        let stack = Box::pin(NetStack {
            waker: None,
            tx,
            rx,
            sink_buf: None,
            _pin: PhantomPinned::default(),
        });

        unsafe {
            OUTPUT_CB_PTR = &*stack as *const NetStack as usize;
        }

        tokio::spawn(async move {
            loop {
                {
                    let _g = LWIP_MUTEX.lock();
                    unsafe { sys_check_timeouts() };
                }
                tokio::time::sleep(time::Duration::from_millis(250)).await;
            }
        });

        stack
    }

    pub fn output(&mut self, pkt: Vec<u8>) {
        if let Err(_) = self.tx.try_send(pkt) {
            // log::trace!("try send stack output pkt failed: {}", e);
        }
        if let Some(waker) = self.waker.as_ref() {
            waker.wake_by_ref();
        }
    }
}

impl Drop for NetStack {
    fn drop(&mut self) {
        log::trace!("drop netstack");
        unsafe {
            let _g = LWIP_MUTEX.lock();
            OUTPUT_CB_PTR = 0x0;
        };
    }
}

impl Stream for NetStack {
    type Item = io::Result<Vec<u8>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let me = unsafe { self.get_unchecked_mut() };
        match me.rx.poll_recv(cx) {
            Poll::Ready(Some(pkt)) => Poll::Ready(Some(Ok(pkt))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                me.waker.replace(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl Sink<Vec<u8>> for NetStack {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let me = unsafe { self.get_unchecked_mut() };
        if me.sink_buf.is_none() {
            Poll::Ready(Ok(()))
        } else {
            unsafe { Pin::new_unchecked(me) }.poll_flush(cx)
        }
    }

    fn start_send(self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        unsafe { self.get_unchecked_mut() }.sink_buf.replace(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let Some(item) = unsafe { self.get_unchecked_mut() }.sink_buf.take() {
            if item.is_empty() {
                return Poll::Ready(Ok(()));
            }
            unsafe {
                let _g = LWIP_MUTEX.lock();

                let pbuf = pbuf_alloc(pbuf_layer_PBUF_RAW, item.len() as u16_t, pbuf_type_PBUF_RAM);
                if pbuf.is_null() {
                    log::trace!("pbuf_alloc null alloc");
                    return Poll::Pending;
                }
                pbuf_take(
                    pbuf,
                    item.as_ptr() as *const raw::c_void,
                    item.len() as u16_t,
                );

                if let Some(input_fn) = (*netif_list).input {
                    let err = input_fn(pbuf, netif_list);
                    if err == err_enum_t_ERR_OK as err_t {
                        Poll::Ready(Ok(()))
                    } else {
                        pbuf_free(pbuf);
                        Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            format!("input error: {}", err),
                        )))
                    }
                } else {
                    pbuf_free(pbuf);
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "input fn not set",
                    )))
                }
            }
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
