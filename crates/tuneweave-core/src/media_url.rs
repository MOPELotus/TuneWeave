//! URL syntax shared by media addresses returned by fixed official producers.
//! This is not authorization for forwarding credentials or fetching arbitrary user URLs.
use crate::{ErrorCode, Result, TuneWeaveError};
use std::net::IpAddr;
use url::Url;

pub fn validate_upstream_media_url(value: &str) -> Result<Url> {
    let invalid = |reason| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("upstream media URL is invalid: {reason}"),
        )
    };
    if value.is_empty()
        || value.len() > 16 * 1024
        || value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err(invalid(
            "empty, oversized or contains whitespace/control characters",
        ));
    }
    let url = Url::parse(value)
        .map_err(|error| invalid("URL parser rejected the address").with_source(&error))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("expected HTTP(S) without userinfo or fragment"));
    }
    let host = url.host_str().unwrap_or_default().trim_end_matches('.');
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err(invalid("local host is not a public CDN"));
    }
    if let Some(ip) = url.host().and_then(|host| match host {
        url::Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
        url::Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
        _ => None,
    }) {
        let private = match ip {
            IpAddr::V4(ip) => {
                ip.is_private()
                    || ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || ip.is_broadcast()
                    || ip.octets()[0] == 0
                    || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
            }
            IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || (ip.segments()[0] & 0xfe00) == 0xfc00
                    || (ip.segments()[0] & 0xffc0) == 0xfe80
                    || ip.to_ipv4_mapped().is_some_and(|ip| {
                        ip.is_loopback()
                            || ip.is_private()
                            || ip.is_link_local()
                            || ip.is_unspecified()
                    })
            }
        };
        if private {
            return Err(invalid("local/private address is not a public CDN"));
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn producer_urls_accept_rotating_hosts_ports_paths_and_signatures() {
        for value in [
            "https://new-cdn.example:4483/new/media?id=1&Signature=opaque",
            "http://new-cdn.example:8080/stream.mp3",
            "https://203.0.113.1:9443/media",
        ] {
            assert!(validate_upstream_media_url(value).is_ok(), "{value}");
        }
        for value in [
            "file:///etc/passwd",
            "https://user:secret@cdn.example/media",
            "https://cdn.example/media#fragment",
            "https://127.0.0.1/media",
            "http://169.254.169.254/metadata",
            "https://[::ffff:127.0.0.1]/media",
            "https://localhost/media",
            "https://10.0.0.1/media",
            "https://cdn.example/a\nb",
        ] {
            assert!(validate_upstream_media_url(value).is_err(), "{value}");
        }
    }
}
