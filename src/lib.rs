mod lwip;
mod mutex;
mod output;
mod stack;
mod tcp_listener;
mod tcp_stream;
mod tcp_stream_context;
mod udp;
mod util;

pub(crate) static LWIP_MUTEX: mutex::AtomicMutex = mutex::AtomicMutex::new();
pub(crate) use mutex::AtomicMutexGuard as LWIPMutexGuard;

pub use stack::NetStack;
pub use tcp_listener::TcpListener;
pub use tcp_stream::TcpStream;
pub use udp::UdpSocket;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("LwIP error ({0})")]
    LwIP(i8),
}
