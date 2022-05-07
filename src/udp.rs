use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    os::raw,
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::stream::Stream;
use futures::task::{Context, Poll, Waker};
use futures::StreamExt;
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
    let listener = unsafe { &mut *(arg as *mut UdpSocket) };
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
            queue.push_back(((&buf[..n]).to_vec(), src_addr, dst_addr));
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

fn send_udp(
    lwip_mutex: &Arc<LWIPMutex>,
    src_addr: &SocketAddr,
    dst_addr: &SocketAddr,
    pcb: usize,
    data: &[u8],
) -> io::Result<()> {
    unsafe {
        let _g = lwip_mutex.lock();
        let data_ptr = data as *const [u8] as *mut [u8] as *mut raw::c_void;
        if data_ptr.is_null() {
            return Err(io::Error::new(io::ErrorKind::Other, "data already freed"));
        }
        let pbuf = pbuf_alloc_reference(data_ptr, data.len() as u16_t, pbuf_type_PBUF_REF);
        let src_ip = match util::to_ip_addr_t(&src_addr.ip()) {
            Ok(v) => v,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Convert address failed",
                ))
            }
        };
        let dst_ip = match util::to_ip_addr_t(&dst_addr.ip()) {
            Ok(v) => v,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Convert address failed",
                ))
            }
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
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("udp_sendto faied: {}", err),
            ));
        }
        pbuf_free(pbuf);
        Ok(())
    }
}

pub struct UdpSocket {
    lwip_mutex: Arc<LWIPMutex>,
    pcb: usize,
    waker: Arc<Mutex<Option<Waker>>>,
    queue: Arc<Mutex<VecDeque<(Vec<u8>, SocketAddr, SocketAddr)>>>,
}

impl UdpSocket {
    pub(crate) fn new(lwip_mutex: Arc<LWIPMutex>) -> Box<Self> {
        unsafe {
            let pcb = udp_new();
            let socket = Box::new(Self {
                lwip_mutex,
                pcb: pcb as usize,
                waker: Arc::new(Mutex::new(None)),
                queue: Arc::new(Mutex::new(VecDeque::new())),
            });
            let err = udp_bind(pcb, &ip_addr_any_type, 0);
            if err != err_enum_t_ERR_OK as err_t {
                panic!("{}", format!("bind udp error: {}", err));
            }
            let arg = &*socket as *const UdpSocket as *mut raw::c_void;
            udp_recv(pcb, Some(udp_recv_cb), arg);
            socket
        }
    }

    pub fn split(self: Box<Self>) -> (SendHalf, RecvHalf) {
        (
            SendHalf {
                lwip_mutex: self.lwip_mutex.clone(),
                pcb: self.pcb,
            },
            RecvHalf { socket: self },
        )
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
    type Item = io::Result<(Vec<u8>, SocketAddr, SocketAddr)>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.queue.lock() {
            Ok(mut queue) => {
                if let Some(pkt) = queue.pop_front() {
                    return Poll::Ready(Some(Ok(pkt)));
                }
            }
            Err(e) => {
                return Poll::Ready(Some(Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Mutex error: {}", e),
                ))));
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
            Err(e) => {
                return Poll::Ready(Some(Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Mutex error: {}", e),
                ))));
            }
        }
        Poll::Pending
    }
}

pub struct SendHalf {
    pub(crate) lwip_mutex: Arc<LWIPMutex>,
    pub(crate) pcb: usize,
}

impl SendHalf {
    pub fn send_to(
        &self,
        data: &[u8],
        src_addr: &SocketAddr,
        dst_addr: &SocketAddr,
    ) -> io::Result<()> {
        send_udp(&self.lwip_mutex, src_addr, dst_addr, self.pcb, data)
    }
}

pub struct RecvHalf {
    pub(crate) socket: Box<UdpSocket>,
}

impl RecvHalf {
    pub async fn recv_from(&mut self) -> io::Result<(Vec<u8>, SocketAddr, SocketAddr)> {
        self.socket.next().await.unwrap()
    }
}
