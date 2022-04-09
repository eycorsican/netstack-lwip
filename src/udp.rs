use std::{
    collections::VecDeque,
    net::SocketAddr,
    os::raw,
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::stream::Stream;
use futures::task::{Context, Poll, Waker};
use log::*;

use super::lwip::*;
use super::util;
use super::LWIPMutex;

pub extern "C" fn udp_recv_cb(
    arg: *mut raw::c_void,
    _pcb: *mut udp_pcb,
    p: *mut pbuf,
    addr: *const ip_addr_t,
    port: u16_t,
    dst_addr: *const ip_addr_t,
    dst_port: u16_t,
) {
    let listener = unsafe { &mut *(arg as *mut UdpListener) };
    let src_addr = unsafe {
        match util::to_socket_addr(&*addr, port) {
            Ok(a) => a,
            Err(_) => return,
        }
    };
    let dst_addr = unsafe {
        match util::to_socket_addr(&*dst_addr, dst_port) {
            Ok(a) => a,
            Err(_) => return,
        }
    };

    let tot_len = unsafe { (*p).tot_len };
    let n = tot_len as usize;
    let mut buf = Vec::<u8>::with_capacity(n);
    unsafe {
        pbuf_copy_partial(p, buf.as_mut_ptr() as *mut raw::c_void, tot_len, 0);
        buf.set_len(n);
        pbuf_free(p);
    }

    match listener.queue.lock() {
        Ok(mut queue) => {
            let pkt = ((&buf[..n]).to_vec(), src_addr, dst_addr);
            queue.push_back(pkt);
            match listener.waker.lock() {
                Ok(waker) => {
                    if let Some(waker) = waker.as_ref() {
                        waker.wake_by_ref();
                    }
                }
                Err(err) => {
                    error!("udp waker lock waker failed {:?}", err);
                }
            }
        }
        Err(err) => {
            error!("udp listener lock queue failed {:?}", err);
        }
    }
}

pub fn send_udp(
    lwip_mutex: Arc<LWIPMutex>,
    src_addr: &SocketAddr,
    dst_addr: &SocketAddr,
    pcb: usize,
    data: &[u8],
) {
    unsafe {
        let _g = lwip_mutex.lock();
        let pbuf = pbuf_alloc_reference(
            data as *const [u8] as *mut [u8] as *mut raw::c_void,
            data.len() as u16_t,
            pbuf_type_PBUF_ROM,
        );
        let src_ip = match util::to_ip_addr_t(&src_addr.ip()) {
            Ok(v) => v,
            Err(_) => return,
        };
        let dst_ip = match util::to_ip_addr_t(&dst_addr.ip()) {
            Ok(v) => v,
            Err(_) => return,
        };
        let err = udp_sendto(
            pcb as *mut udp_pcb,
            pbuf,
            &dst_ip as *const ip_addr_t,
            dst_addr.port() as u16_t,
            &src_ip as *const ip_addr_t,
            src_addr.port() as u16_t,
        );
        if err != err_enum_t_ERR_OK as err_t {
            warn!("udp_sendto error: {}", err);
        }
        pbuf_free(pbuf);
    }
}

pub struct UdpListener {
    pcb: *mut udp_pcb,
    waker: Arc<Mutex<Option<Waker>>>,
    queue: Arc<Mutex<VecDeque<(Vec<u8>, SocketAddr, SocketAddr)>>>,
}

impl UdpListener {
    pub fn new() -> Self {
        unsafe {
            let pcb = udp_new();
            let listener = UdpListener {
                pcb,
                waker: Arc::new(Mutex::new(None)),
                queue: Arc::new(Mutex::new(VecDeque::new())),
            };
            let err = udp_bind(pcb, &ip_addr_any_type, 0);
            if err != err_enum_t_ERR_OK as err_t {
                panic!("{}", format!("bind udp error: {}", err));
            }
            let arg = &listener as *const UdpListener as *mut raw::c_void;
            udp_recv(pcb, Some(udp_recv_cb), arg);
            listener
        }
    }

    pub fn pcb(&self) -> usize {
        self.pcb as usize
    }
}

unsafe impl Sync for UdpListener {}
unsafe impl Send for UdpListener {}

impl Drop for UdpListener {
    fn drop(&mut self) {
        unsafe {
            udp_recv(self.pcb, None, std::ptr::null_mut());
            udp_remove(self.pcb);
        }
    }
}

impl Stream for UdpListener {
    type Item = (Vec<u8>, SocketAddr, SocketAddr);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.queue.lock() {
            Ok(mut queue) => {
                if let Some(sess) = queue.pop_front() {
                    return Poll::Ready(Some(sess));
                }
            }
            Err(err) => {
                error!("sess poll lock queue failed: {:?}", err);
            }
        }
        match self.waker.lock() {
            Ok(mut waker) => {
                if let Some(waker_ref) = waker.as_ref() {
                    if !waker_ref.will_wake(cx.waker()) {
                        waker.replace(cx.waker().clone());
                    }
                } else {
                    waker.replace(cx.waker().clone());
                }
            }
            Err(err) => {
                error!("sess poll lock waker failed: {:?}", err);
            }
        }
        Poll::Pending
    }
}
