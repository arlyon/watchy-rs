use chrono::{NaiveDateTime, Timelike};
use embassy_futures::select;
use embassy_net::{udp::UdpSocket, IpAddress};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_nal_async::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use esp_hal::rtc_cntl::Rtc;

use crate::sticky_signal::StickySignal;
use esp_wifi::wifi::ipv4::ToSocketAddrs;

use futures::Stream;
use sntpc::{NtpContext, NtpResult, NtpTimestampGenerator};

/// The estimated offset between system time and real time.
///
/// This number is usually determined using an ntp server and
/// updated later.
static TIME_OFFSET: StickySignal<CriticalSectionRawMutex, u64, 4> =
    StickySignal::new_with_name("time_offset");

/// A time struct. This is initialized to empty and is updated when
/// the time changes.
#[derive(Clone, Copy)]
pub struct GlobalTime {
    rtc: &'static Rtc<'static>,
}

impl GlobalTime {
    pub fn new(rtc: &'static Rtc) -> Self {
        Self { rtc }
    }

    pub fn init_offset(&self, offset_micros: u64) {
        TIME_OFFSET.signal(offset_micros);
    }

    pub fn init_time(&self, seconds: u32, seconds_fraction: u32) {
        // a single second fraction is 0.2 ns
        let current_time = NaiveDateTime::from_timestamp(seconds.into(), seconds_fraction / 5);
        defmt::info!(
            "time is {}:{}:{}",
            current_time.hour(),
            current_time.minute(),
            current_time.second()
        );
        self.rtc.set_current_time(current_time);
    }

    /// Get the time based on the system time + offset
    pub fn get_time(&self) -> u64 {
        let microseconds = esp_hal::time::now().duration_since_epoch().to_micros();

        let offset = TIME_OFFSET.peek().unwrap_or_default();

        defmt::info!(
            "time is {} + {} = {}",
            microseconds,
            offset,
            microseconds + offset
        );
        microseconds + offset
    }

    /// Produces a stream that terminates either when the offset is updated,
    /// or never.
    ///
    /// TODO: make sure the first one starts on the minute
    pub fn minutes(&self) -> impl Stream<Item = u64> + '_ {
        let ticker = embassy_time::Ticker::every(embassy_time::Duration::from_secs(60));
        futures::stream::unfold(ticker, move |mut ticker| async move {
            match select::select(ticker.next(), TIME_OFFSET.wait("time offset updated")).await {
                select::Either::First(()) => Some((self.get_time(), ticker)),
                select::Either::Second(_) => {
                    defmt::info!("offset changed, exiting");
                    None
                }
            }
        })
    }
}

#[derive(Copy, Clone, Default)]
struct StdTimestampGen {
    duration: core::time::Duration,
}

impl NtpTimestampGenerator for StdTimestampGen {
    fn init(&mut self) {
        let microseconds = esp_hal::time::now().duration_since_epoch().to_micros();
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
        .inspect_err(|e| {
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
        })
        .ok()
}
