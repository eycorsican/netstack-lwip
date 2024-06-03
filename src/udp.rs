use std::{io, net::SocketAddr, os::raw, pin::Pin};
use std::marker::PhantomPinned;

use futures::stream::Stream;
use futures::task::{Context, Poll, Waker};
use futures::StreamExt;
use log::{error, warn};
use tokio::sync::mpsc::{channel, Receiver, Sender};

use super::lwip::*;
use super::util;
use crate::Error;

pub unsafe extern "C" fn udp_recv_cb(
    arg: *mut raw::c_void,
    _pcb: *mut udp_pcb,
    p: *mut pbuf,
    addr: *const ip_addr_t,
    port: u16_t,
    dst_addr: *const ip_addr_t,
    dst_port: u16_t,
) {
    if arg.is_null() {
        warn!("udp socket has been closed");
        return;
    }
    let socket = &mut *(arg as *mut UdpSocket);
    let src_addr = util::to_socket_addr(&*addr, port);
    let dst_addr = util::to_socket_addr(&*dst_addr, dst_port);
    let tot_len = std::ptr::read_unaligned(p).tot_len;
    let mut buf = Vec::with_capacity(tot_len as usize);
    pbuf_copy_partial(p, buf.as_mut_ptr() as *mut _, tot_len, 0);
    buf.set_len(tot_len as usize);
    pbuf_free(p);
    if let Err(_) = socket.tx.try_send((buf, src_addr, dst_addr)) {
        // log::trace!("try send udp pkt failed (netstack): {}", e);
    }
    if let Some(waker) = socket.waker.as_ref() {
        waker.wake_by_ref();
    }
}

fn send_udp(
    src_addr: &SocketAddr,
    dst_addr: &SocketAddr,
    pcb: usize,
    data: &[u8],
) -> io::Result<()> {
    unsafe {
        let _g = super::LWIP_MUTEX.lock();
        let pbuf =
            pbuf_alloc_reference(data.as_ptr() as *mut _, data.len() as _, pbuf_type_PBUF_REF);
        let src_ip = util::to_ip_addr_t(src_addr.ip());
        let dst_ip = util::to_ip_addr_t(dst_addr.ip());
        let err = udp_sendto(
            pcb as *mut udp_pcb,
            pbuf,
            &dst_ip as *const _,
            dst_addr.port(),
            &src_ip as *const _,
            src_addr.port(),
        );
        pbuf_free(pbuf);
        if err != err_enum_t_ERR_OK as err_t {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("udp_sendto error: {}", err),
            ));
        }
        Ok(())
    }
}

type UdpPkt = (Vec<u8>, SocketAddr, SocketAddr);

pub struct UdpSocket {
    pcb: usize,
    waker: Option<Waker>,
    tx: Sender<UdpPkt>,
    rx: Receiver<UdpPkt>,
    _pin: PhantomPinned
}

impl UdpSocket {
    pub(crate) fn new(buffer_size: usize) -> Result<Pin<Box<Self>>, Error> {
        unsafe {
            let pcb = udp_new();
            let (tx, rx): (Sender<UdpPkt>, Receiver<UdpPkt>) = channel(buffer_size);
            let socket = Box::pin(Self {
                pcb: pcb as usize,
                waker: None,
                tx,
                rx,
                _pin: PhantomPinned::default()
            });
            let err = udp_bind(pcb, &ip_addr_any_type, 0);
            if err != err_enum_t_ERR_OK as err_t {
                error!("bind UDP failed: {}", err);
                return Err(Error::LwIP(err));
            }
            let arg = &*socket as *const UdpSocket as *mut raw::c_void;
            udp_recv(pcb, Some(udp_recv_cb), arg);
            Ok(socket)
        }
    }

    pub fn split(self: Pin<Box<Self>>) -> (SendHalf, RecvHalf) {
        (SendHalf { pcb: self.pcb }, RecvHalf { socket: self })
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        unsafe {
            udp_recv(self.pcb as *mut udp_pcb, None, std::ptr::null_mut());
            udp_remove(self.pcb as *mut udp_pcb);
        }
    }
}

impl Stream for UdpSocket {
    type Item = UdpPkt;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = unsafe { self.get_unchecked_mut() };

        match this.rx.poll_recv(cx) {
            Poll::Ready(Some(pkt)) => Poll::Ready(Some(pkt)),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                this.waker.replace(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

pub struct SendHalf {
    pub(crate) pcb: usize,
}

impl SendHalf {
    pub fn send_to(
        &self,
        data: &[u8],
        src_addr: &SocketAddr,
        dst_addr: &SocketAddr,
    ) -> io::Result<()> {
        send_udp(src_addr, dst_addr, self.pcb, data)
    }
}

pub struct RecvHalf {
    pub(crate) socket: Pin<Box<UdpSocket>>,
}

impl RecvHalf {
    pub async fn recv_from(&mut self) -> io::Result<UdpPkt> {
        match self.socket.next().await {
            Some(pkt) => Ok(pkt),
            None => Err(io::Error::new(
                io::ErrorKind::Other,
                format!("recv_from udp socket faied: tx closed"),
            )),
        }
    }
}

impl Stream for RecvHalf {
    type Item = UdpPkt;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.socket).poll_next(cx)
    }
}
