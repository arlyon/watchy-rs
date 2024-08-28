use embassy_net::{udp::UdpSocket, IpAddress};
use embedded_nal_async::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

use esp_wifi::wifi::ipv4::ToSocketAddrs;

use sntpc::{NtpContext, NtpResult, NtpTimestampGenerator};

#[derive(Copy, Clone, Default)]
struct StdTimestampGen {
    duration: core::time::Duration,
}

impl NtpTimestampGenerator for StdTimestampGen {
    fn init(&mut self) {
        let microseconds = esp_hal::time::current_time()
            .duration_since_epoch()
            .to_micros();
        self.duration = core::time::Duration::from_micros(microseconds);
    }

    fn timestamp_sec(&self) -> u64 {
        self.duration.as_secs()
    }

    fn timestamp_subsec_micros(&self) -> u32 {
        self.duration.subsec_micros()
    }
}

const NTP_SERVER: (u8, u8, u8, u8) = (185, 83, 169, 27);
const NTP_PORT: u16 = 123;

struct EspWifiUdpSocket<'a> {
    socket: UdpSocket<'a>,
}

impl<'a> EspWifiUdpSocket<'a> {
    fn new(socket: UdpSocket<'a>) -> Self {
        Self { socket }
    }
}

impl sntpc::async_impl::NtpUdpSocket for EspWifiUdpSocket<'_> {
    async fn send_to<T: ToSocketAddrs + Send>(&self, buf: &[u8], addr: T) -> sntpc::Result<usize> {
        defmt::info!("send");

        let addrs = addr.to_socket_addrs().unwrap().next().unwrap();
        let port = addrs.port();
        let IpAddr::V4(addr) = addrs.ip() else {
            panic!("we do not support ipv6");
        };
        let [a, b, c, d] = addr.octets();
        self.socket
            .send_to(
                buf,
                (
                    smoltcp::wire::IpAddress::from(smoltcp::wire::Ipv4Address::new(a, b, c, d)),
                    port,
                ),
            )
            .await
            .map_err(|e| {
                defmt::error!("error during time send: {}", e);
                sntpc::Error::Network
            })?;
        Ok(buf.len())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> sntpc::Result<(usize, SocketAddr)> {
        defmt::info!("recv");
        self.socket
            .recv_from(buf)
            .await
            .map(|(bytes, meta)| {
                let IpAddress::Ipv4(smoltcp::wire::Ipv4Address([a, b, c, d])) = meta.endpoint.addr;
                (
                    bytes,
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), meta.endpoint.port),
                )
            })
            .map_err(|e| {
                defmt::error!("error during time recv: {}", e);
                sntpc::Error::Network
            })
    }
}

impl core::fmt::Debug for EspWifiUdpSocket<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "EspWifiUpdSocket")
    }
}

pub async fn get_time(socket: UdpSocket<'_>) -> Option<NtpResult> {
    let server_socket_addr = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(NTP_SERVER.0, NTP_SERVER.1, NTP_SERVER.2, NTP_SERVER.3),
        NTP_PORT,
    ));
    let socket = EspWifiUdpSocket::new(socket);

    let context = NtpContext::new(StdTimestampGen::default());
    sntpc::async_impl::get_time(server_socket_addr, socket, context)
        .await
        .map_err(|e| {
            defmt::error!(
                "failed to get time {}",
                match e {
                    sntpc::Error::IncorrectOriginTimestamp => "incorrect origin",
                    sntpc::Error::IncorrectMode => "incorrect mode",
                    sntpc::Error::IncorrectLeapIndicator => "incorrect leap",
                    sntpc::Error::IncorrectResponseVersion => "incorrect response",
                    sntpc::Error::IncorrectStratumHeaders => "incorrect stratum",
                    sntpc::Error::IncorrectPayload => "incorrect payload",
                    sntpc::Error::Network => "network",
                    sntpc::Error::AddressResolve => "address resolve",
                    sntpc::Error::ResponseAddressMismatch => "response mismatch",
                    _ => "unknown",
                }
            );
            e
        })
        .ok()
}
