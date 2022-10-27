use std::net::{IpAddr, SocketAddr};

use super::lwip::*;

// TODO Support Ipv6

pub fn to_socket_addr(addr: &ip_addr_t, port: u16_t) -> SocketAddr {
    unsafe {
        match addr.type_ {
            // Ipv4
            0 => SocketAddr::new(IpAddr::V4(addr.u_addr.ip4.addr.swap_bytes().into()), port),
            _ => unimplemented!(),
        }
    }
}

pub fn to_ip_addr_t(ip: IpAddr) -> ip_addr_t {
    unsafe {
        match ip {
            IpAddr::V4(ip4) => {
                ip_addr_t {
                    u_addr: ip_addr__bindgen_ty_1 {
                        ip4: ip4_addr {
                            addr: u32::from(ip4).swap_bytes(),
                        },
                    },
                    type_: 0, // Ipv4
                }
            }
            _ => unimplemented!(),
        }
    }
}
