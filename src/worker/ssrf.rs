use reqwest::{
    Client, StatusCode,
    dns::{Addrs, Name, Resolve, Resolving},
    header::{HeaderMap, HeaderValue, LOCATION},
};
use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use url::Url;

pub const MAX_BODY: usize = 10 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

/// Validating resolver: the addresses reqwest actually connects to are the ones
/// checked here, so a second DNS answer cannot bypass the SSRF blocklist.
pub struct SafeResolver;

impl Resolve for SafeResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(boxed)?
                .collect::<Vec<_>>();
            if addrs.is_empty() {
                return Err(boxed_message("Could not resolve that host."));
            }
            if addrs.iter().any(|addr| blocked(addr.ip())) {
                return Err(boxed_message("That address is not allowed."));
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

fn boxed(error: impl Error + Send + Sync + 'static) -> Box<dyn Error + Send + Sync> {
    Box::new(error)
}

fn boxed_message(message: &'static str) -> Box<dyn Error + Send + Sync> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message,
    ))
}

fn allow_loopback() -> bool {
    cfg!(any(test, feature = "ssrf-test-exception"))
}

fn blocked(ip: IpAddr) -> bool {
    is_blocked(ip) && !(allow_loopback() && ip.is_loopback())
}

pub struct Fetched {
    pub final_url: Url,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

/// Rejects any URL whose host resolves to a non-public address.
pub async fn check_url(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https URLs are allowed.".into());
    }
    let host = url.host_str().ok_or("URL has no host")?;
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| "Could not resolve that host.".to_string())?;
    let mut resolved = false;
    for addr in addrs {
        resolved = true;
        if blocked(addr.ip()) {
            return Err("That address is not allowed.".into());
        }
    }
    if !resolved {
        return Err("Could not resolve that host.".into());
    }
    Ok(())
}

pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_unspecified()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_multicast()
        || ip.to_ipv4_mapped().map(is_blocked_v4).unwrap_or(false)
}

/// GET with manual redirects: every hop is SSRF-checked and the body is capped.
pub async fn get(client: &Client, url: &Url, extra_headers: &HeaderMap) -> Result<Fetched, String> {
    let mut current = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        check_url(&current).await?;
        let response = client
            .get(current.clone())
            .headers(extra_headers.clone())
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;
        let status = response.status();
        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("Redirect without a location")?;
            current = current
                .join(location)
                .map_err(|_| "Invalid redirect location".to_string())?;
            continue;
        }
        if !status.is_success() {
            return Ok(Fetched {
                final_url: current,
                status,
                headers: response.headers().clone(),
                body: Vec::new(),
            });
        }
        let headers = response.headers().clone();
        let mut body = Vec::new();
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err("Response body is too large.".into());
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(Fetched {
            final_url: current,
            status,
            headers,
            body,
        });
    }
    Err("Too many redirects.".into())
}

pub fn header_value(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .map(str::to_string)
}
