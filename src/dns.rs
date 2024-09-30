use embedded_nal_async::IpAddr;
use esp_println::println;

/// A simple dns resolver that only supports IP addresses
pub struct StaticDns;

impl embedded_nal_async::Dns for StaticDns {
    type Error = ();

    async fn get_host_by_name(
        &self,
        _host: &str,
        _addr_type: embedded_nal_async::AddrType,
    ) -> Result<embedded_nal_async::IpAddr, Self::Error> {
        Ok(IpAddr::from(parse_ip4v(_host)))
    }

    async fn get_host_by_address(
        &self,
        _addr: embedded_nal_async::IpAddr,
        _result: &mut [u8],
    ) -> Result<usize, Self::Error> {
        println!("{:?}", _addr);
        Err(())
    }
}

fn parse_ip4v(input: &str) -> [u8; 4] {
    let (p1, input) = input.split_once('.').unwrap();
    let (p2, input) = input.split_once('.').unwrap();
    let (p3, p4) = input.split_once('.').unwrap();

    [
        p1.parse().unwrap(),
        p2.parse().unwrap(),
        p3.parse().unwrap(),
        p4.parse().unwrap(),
    ]
}
