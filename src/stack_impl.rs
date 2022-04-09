use std::{
    io,
    os::raw,
    pin::Pin,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Once,
    },
    time,
};

use futures::task::{Context, Poll, Waker};
use log::*;
use tokio::{
    self,
    io::{AsyncRead, AsyncWrite, ReadBuf},
};

use super::lwip::*;
use super::output::{output_ip4, output_ip6, OUTPUT_CB_PTR};
use super::LWIPMutex;

static LWIP_INIT: Once = Once::new();

pub struct NetStackImpl {
    pub lwip_mutex: Arc<LWIPMutex>,
    waker: Option<Waker>,
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
}

impl NetStackImpl {
    pub fn new(lwip_mutex: Arc<LWIPMutex>) -> Box<Self> {
        LWIP_INIT.call_once(|| unsafe { lwip_init() });

        unsafe {
            (*netif_list).output = Some(output_ip4);
            (*netif_list).output_ip6 = Some(output_ip6);
            (*netif_list).mtu = 1500;
        }

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();

        let stack = Box::new(NetStackImpl {
            lwip_mutex,
            waker: None,
            tx,
            rx,
        });

        unsafe {
            OUTPUT_CB_PTR = &*stack as *const NetStackImpl as usize;
        }

        let lwip_mutex = stack.lwip_mutex.clone();
        tokio::spawn(async move {
            loop {
                {
                    let _g = lwip_mutex.lock();
                    unsafe { sys_check_timeouts() };
                }
                tokio::time::sleep(time::Duration::from_millis(250)).await;
            }
        });

        stack
    }

    pub fn output(&mut self, pkt: Vec<u8>) -> io::Result<usize> {
        let n = pkt.len();
        if let Err(_) = self.tx.send(pkt) {
            return Ok(0);
        }
        if let Some(waker) = self.waker.as_ref() {
            waker.wake_by_ref();
            return Ok(n);
        }
        Ok(0)
    }
}

impl Drop for NetStackImpl {
    fn drop(&mut self) {
        log::trace!("drop netstack");
        unsafe {
            let _g = self.lwip_mutex.lock();
            OUTPUT_CB_PTR = 0x0;
        };
    }
}

impl AsyncRead for NetStackImpl {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        match self.rx.try_recv() {
            Ok(pkt) => {
                if pkt.len() > buf.remaining() {
                    warn!("truncated pkt, short buf");
                }
                buf.put_slice(&pkt);
                Poll::Ready(Ok(()))
            }
            Err(_) => {
                if let Some(waker) = self.waker.as_ref() {
                    if !waker.will_wake(cx.waker()) {
                        self.waker.replace(cx.waker().clone());
                    }
                } else {
                    self.waker.replace(cx.waker().clone());
                }
                Poll::Pending
            }
        }
    }
}

impl AsyncWrite for NetStackImpl {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        unsafe {
            let _g = self.lwip_mutex.lock();

            let pbuf = pbuf_alloc(pbuf_layer_PBUF_RAW, buf.len() as u16_t, pbuf_type_PBUF_RAM);
            if pbuf.is_null() {
                return Poll::Pending;
            }
            pbuf_take(pbuf, buf.as_ptr() as *const raw::c_void, buf.len() as u16_t);

            if let Some(input_fn) = (*netif_list).input {
                let err = input_fn(pbuf, netif_list);
                if err == err_enum_t_ERR_OK as err_t {
                    Poll::Ready(Ok(buf.len()))
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
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
