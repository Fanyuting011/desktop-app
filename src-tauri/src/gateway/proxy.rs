use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::network_log::{NetworkLogBuffer, NetworkLogEntry};

#[derive(Debug, Clone)]
pub enum UpstreamKind {
    Http { hostport: String },
    Socks5 { hostport: String },
}

impl UpstreamKind {
    pub fn parse(input: &str) -> Result<Option<Self>, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("socks5://")
            || lower.starts_with("socks5h://")
            || lower.starts_with("socks://")
        {
            let idx = trimmed.find("://").map(|i| i + 3).unwrap_or(0);
            let hostport = trimmed[idx..].trim_end_matches('/').to_string();
            if hostport.is_empty() {
                return Err("上游 SOCKS 地址无效".into());
            }
            return Ok(Some(Self::Socks5 { hostport }));
        }

        let hostport = if lower.starts_with("http://") || lower.starts_with("https://") {
            let idx = trimmed.find("://").map(|i| i + 3).unwrap_or(0);
            trimmed[idx..].trim_end_matches('/').to_string()
        } else {
            trimmed.trim_end_matches('/').to_string()
        };
        if hostport.is_empty() {
            return Err("上游 HTTP 代理地址无效".into());
        }
        let hp = if hostport.contains(':') {
            hostport
        } else {
            format!("{hostport}:7890")
        };
        Ok(Some(Self::Http { hostport: hp }))
    }

    pub fn display(&self) -> String {
        match self {
            Self::Http { hostport } => format!("http://{hostport}"),
            Self::Socks5 { hostport } => format!("socks5://{hostport}"),
        }
    }
}
pub struct ProxyHandles {
    pub stop_tx: watch::Sender<bool>,
    pub http_addr: String,
    pub socks_addr: String,
    running: Arc<AtomicBool>,
}

impl ProxyHandles {
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
        self.running.store(false, Ordering::SeqCst);
    }
}

pub async fn start_local_proxies(
    http_port: u16,
    socks_port: u16,
    upstream: Option<UpstreamKind>,
    profile_id: String,
    net_log: Arc<NetworkLogBuffer>,
) -> Result<ProxyHandles, String> {
    let http_listener = TcpListener::bind(("127.0.0.1", http_port))
        .await
        .map_err(|e| format!("绑定 HTTP 代理失败 127.0.0.1:{http_port}: {e}"))?;
    let socks_listener = TcpListener::bind(("127.0.0.1", socks_port))
        .await
        .map_err(|e| format!("绑定 SOCKS 代理失败 127.0.0.1:{socks_port}: {e}"))?;

    let (stop_tx, stop_rx_http) = watch::channel(false);
    let stop_rx_socks = stop_tx.subscribe();
    let running = Arc::new(AtomicBool::new(true));
    let upstream_http = Arc::new(upstream.clone());
    let upstream_socks = upstream_http.clone();
    let profile_id = Arc::new(profile_id);

    let running_http = running.clone();
    let http_profile_id = profile_id.clone();
    let http_net_log = net_log.clone();
    tokio::spawn(async move {
        run_http_proxy(
            http_listener,
            stop_rx_http,
            upstream_http,
            http_profile_id,
            http_net_log,
        )
        .await;
        running_http.store(false, Ordering::SeqCst);
    });

    let running_socks = running.clone();
    tokio::spawn(async move {
        run_socks_proxy(
            socks_listener,
            stop_rx_socks,
            upstream_socks,
            profile_id,
            net_log,
        )
        .await;
        running_socks.store(false, Ordering::SeqCst);
    });

    Ok(ProxyHandles {
        stop_tx,
        http_addr: format!("127.0.0.1:{http_port}"),
        socks_addr: format!("127.0.0.1:{socks_port}"),
        running,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn push_network_log(
    net_log: &NetworkLogBuffer,
    profile_id: &str,
    protocol: &str,
    target: &str,
    error: Option<String>,
) {
    net_log.push(NetworkLogEntry {
        id: uuid::Uuid::new_v4().to_string(),
        ts_ms: now_ms(),
        profile_id: profile_id.to_string(),
        protocol: protocol.to_string(),
        target: target.to_string(),
        ok: error.is_none(),
        error,
    });
}

async fn dial_target(
    target_hostport: &str,
    upstream: Option<&UpstreamKind>,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    match upstream {
        None => Ok(TcpStream::connect(target_hostport).await?),
        Some(UpstreamKind::Http { hostport }) => {
            let mut stream = TcpStream::connect(hostport).await?;
            let req = format!(
                "CONNECT {target_hostport} HTTP/1.1\r\nHost: {target_hostport}\r\nProxy-Connection: Keep-Alive\r\n\r\n"
            );
            stream.write_all(req.as_bytes()).await?;
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await?;
            let resp = String::from_utf8_lossy(&buf[..n]);
            let ok = resp
                .lines()
                .next()
                .map(|l| l.contains(" 200 "))
                .unwrap_or(false);
            if !ok {
                return Err(format!("上游 HTTP 代理 CONNECT 失败: {}", resp.lines().next().unwrap_or("")).into());
            }
            Ok(stream)
        }
        Some(UpstreamKind::Socks5 { hostport }) => {
            socks5_connect(hostport, target_hostport).await
        }
    }
}

async fn socks5_connect(
    proxy_hostport: &str,
    target_hostport: &str,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = TcpStream::connect(proxy_hostport).await?;
    stream.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut resp = [0u8; 2];
    stream.read_exact(&mut resp).await?;
    if resp != [0x05, 0x00] {
        return Err("上游 SOCKS5 握手失败".into());
    }

    let (host, port) = split_host_port(target_hostport)?;
    let mut req = vec![0x05, 0x01, 0x00];
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        req.push(0x01);
        req.extend_from_slice(&ip.octets());
    } else if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        req.push(0x04);
        req.extend_from_slice(&ip.octets());
    } else {
        let hb = host.as_bytes();
        if hb.len() > 255 {
            return Err("域名过长".into());
        }
        req.push(0x03);
        req.push(hb.len() as u8);
        req.extend_from_slice(hb);
    }
    req.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&req).await?;

    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr).await?;
    if hdr[1] != 0x00 {
        return Err(format!("上游 SOCKS5 CONNECT 失败 code={}", hdr[1]).into());
    }
    match hdr[3] {
        0x01 => {
            let mut skip = [0u8; 6];
            stream.read_exact(&mut skip).await?;
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut skip = vec![0u8; len[0] as usize + 2];
            stream.read_exact(&mut skip).await?;
        }
        0x04 => {
            let mut skip = [0u8; 18];
            stream.read_exact(&mut skip).await?;
        }
        _ => return Err("上游 SOCKS5 地址类型未知".into()),
    }
    Ok(stream)
}

fn split_host_port(hostport: &str) -> Result<(String, u16), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(rest) = hostport.strip_prefix('[') {
        let (host, port_part) = rest
            .split_once("]:")
            .ok_or("无效的 IPv6 host:port")?;
        return Ok((host.to_string(), port_part.parse()?));
    }
    let (host, port) = hostport
        .rsplit_once(':')
        .ok_or("无效的 host:port")?;
    Ok((host.to_string(), port.parse()?))
}

async fn run_http_proxy(
    listener: TcpListener,
    mut stop_rx: watch::Receiver<bool>,
    upstream: Arc<Option<UpstreamKind>>,
    profile_id: Arc<String>,
    net_log: Arc<NetworkLogBuffer>,
) {
    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        let up = upstream.clone();
                        let profile_id = profile_id.clone();
                        let net_log = net_log.clone();
                        tokio::spawn(async move {
                            let _ = handle_http_client(
                                stream,
                                up.as_ref().as_ref(),
                                &profile_id,
                                &net_log,
                            )
                            .await;
                        });
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

async fn handle_http_client(
    mut client: TcpStream,
    upstream: Option<&UpstreamKind>,
    profile_id: &str,
    net_log: &NetworkLogBuffer,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = vec![0u8; 8192];
    let n = client.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let header_text = String::from_utf8_lossy(&buf[..n]);
    let first_line = header_text.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let target = parts[1];

    if method.eq_ignore_ascii_case("CONNECT") {
        let hostport = target;
        match dial_target(hostport, upstream).await {
            Ok(remote) => {
                push_network_log(net_log, profile_id, "http", hostport, None);
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await?;
                relay(client, remote).await?;
            }
            Err(error) => {
                push_network_log(
                    net_log,
                    profile_id,
                    "http",
                    hostport,
                    Some(error.to_string()),
                );
                client
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                    .await?;
            }
        }
        return Ok(());
    }

    // Absolute-form HTTP proxy request
    let url = target.to_string();
    let (hostport, path) = parse_absolute_url(&url).ok_or("invalid proxy url")?;

    // If upstream is HTTP proxy, forward the original absolute-form request to it.
    if let Some(UpstreamKind::Http { hostport: up }) = upstream {
        let mut remote = TcpStream::connect(up).await?;
        remote.write_all(&buf[..n]).await?;
        relay(client, remote).await?;
        return Ok(());
    }

    let mut remote = dial_target(&hostport, upstream).await?;
    let mut rewritten = format!("{method} {path} HTTP/1.1\r\n");
    let mut saw_host = false;
    for line in header_text.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("proxy-connection:") {
            continue;
        }
        if lower.starts_with("host:") {
            saw_host = true;
        }
        rewritten.push_str(line);
        rewritten.push_str("\r\n");
    }
    if !saw_host {
        let host_only = hostport.split(':').next().unwrap_or(&hostport);
        rewritten.push_str(&format!("Host: {host_only}\r\n"));
    }
    rewritten.push_str("\r\n");

    let header_end = header_text
        .find("\r\n\r\n")
        .map(|i| i + 4)
        .or_else(|| header_text.find("\n\n").map(|i| i + 2))
        .unwrap_or(n);

    remote.write_all(rewritten.as_bytes()).await?;
    if header_end < n {
        remote.write_all(&buf[header_end..n]).await?;
    }
    relay(client, remote).await?;
    Ok(())
}

fn parse_absolute_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let hostport = if hostport.contains(':') {
        hostport.to_string()
    } else {
        format!("{hostport}:80")
    };
    Some((hostport, path.to_string()))
}

async fn relay(a: TcpStream, b: TcpStream) -> Result<(), std::io::Error> {
    let (mut ar, mut aw) = a.into_split();
    let (mut br, mut bw) = b.into_split();
    let t1 = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut ar, &mut bw).await;
    });
    let t2 = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut br, &mut aw).await;
    });
    let _ = tokio::join!(t1, t2);
    Ok(())
}

async fn run_socks_proxy(
    listener: TcpListener,
    mut stop_rx: watch::Receiver<bool>,
    upstream: Arc<Option<UpstreamKind>>,
    profile_id: Arc<String>,
    net_log: Arc<NetworkLogBuffer>,
) {
    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        let up = upstream.clone();
                        let profile_id = profile_id.clone();
                        let net_log = net_log.clone();
                        tokio::spawn(async move {
                            let _ = handle_socks_client(
                                stream,
                                up.as_ref().as_ref(),
                                &profile_id,
                                &net_log,
                            )
                            .await;
                        });
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

async fn handle_socks_client(
    mut client: TcpStream,
    upstream: Option<&UpstreamKind>,
    profile_id: &str,
    net_log: &NetworkLogBuffer,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = [0u8; 2];
    client.read_exact(&mut buf).await?;
    if buf[0] != 0x05 {
        return Ok(());
    }
    let nmethods = buf[1] as usize;
    let mut methods = vec![0u8; nmethods];
    client.read_exact(&mut methods).await?;
    client.write_all(&[0x05, 0x00]).await?;

    let mut req_hdr = [0u8; 4];
    client.read_exact(&mut req_hdr).await?;
    if req_hdr[0] != 0x05 || req_hdr[1] != 0x01 {
        let _ = client.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
        return Ok(());
    }
    let atyp = req_hdr[3];
    let dest = match atyp {
        0x01 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await?;
            let mut port_buf = [0u8; 2];
            client.read_exact(&mut port_buf).await?;
            let port = u16::from_be_bytes(port_buf);
            format!("{}.{}.{}.{}:{}", ip[0], ip[1], ip[2], ip[3], port)
        }
        0x03 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            client.read_exact(&mut domain).await?;
            let mut port_buf = [0u8; 2];
            client.read_exact(&mut port_buf).await?;
            let port = u16::from_be_bytes(port_buf);
            let host = String::from_utf8_lossy(&domain);
            format!("{host}:{port}")
        }
        0x04 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip).await?;
            let mut port_buf = [0u8; 2];
            client.read_exact(&mut port_buf).await?;
            let port = u16::from_be_bytes(port_buf);
            let addr = std::net::Ipv6Addr::from(ip);
            format!("[{addr}]:{port}")
        }
        _ => {
            let _ = client.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
            return Ok(());
        }
    };

    match dial_target(&dest, upstream).await {
        Ok(remote) => {
            push_network_log(net_log, profile_id, "socks", &dest, None);
            client
                .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            relay(client, remote).await?;
        }
        Err(error) => {
            push_network_log(net_log, profile_id, "socks", &dest, Some(error.to_string()));
            client
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::network_log::{NetworkLogBuffer, NetworkLogEntry};
    use tokio::time::{sleep, Duration};

    async fn unused_port_pair() -> (u16, u16) {
        let first = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let second = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        (
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port(),
        )
    }

    async fn unused_port() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        listener.local_addr().unwrap().port()
    }

    async fn wait_for_log(logs: &NetworkLogBuffer) -> NetworkLogEntry {
        for _ in 0..20 {
            if let Some(entry) = logs.snapshot(None, 1).into_iter().next() {
                return entry;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("network log was not emitted");
    }

    #[tokio::test]
    async fn http_connect_emits_success_log() {
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_addr = target.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = target.accept().await;
        });
        let (http_port, socks_port) = unused_port_pair().await;
        let logs = Arc::new(NetworkLogBuffer::new());
        let proxy = start_local_proxies(
            http_port,
            socks_port,
            None,
            "profile-http".into(),
            logs.clone(),
        )
        .await
        .unwrap();

        let mut client = TcpStream::connect(("127.0.0.1", http_port)).await.unwrap();
        client
            .write_all(
                format!("CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
        let mut response = [0u8; 39];
        let _ = client.read(&mut response).await.unwrap();

        let entry = wait_for_log(&logs).await;
        assert_eq!(entry.profile_id, "profile-http");
        assert_eq!(entry.protocol, "http");
        assert_eq!(entry.target, target_addr.to_string());
        assert!(entry.ok);
        assert!(entry.error.is_none());
        proxy.stop();
    }

    #[tokio::test]
    async fn http_connect_emits_failure_log() {
        let (http_port, socks_port) = unused_port_pair().await;
        let logs = Arc::new(NetworkLogBuffer::new());
        let proxy = start_local_proxies(
            http_port,
            socks_port,
            None,
            "profile-http-fail".into(),
            logs.clone(),
        )
        .await
        .unwrap();
        let target_port = unused_port().await;

        let mut client = TcpStream::connect(("127.0.0.1", http_port)).await.unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{target_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();

        let entry = wait_for_log(&logs).await;
        assert_eq!(entry.protocol, "http");
        assert_eq!(entry.target, format!("127.0.0.1:{target_port}"));
        assert!(!entry.ok);
        assert!(entry.error.is_some());
        proxy.stop();
    }

    #[tokio::test]
    async fn socks_connect_emits_success_log() {
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_addr = target.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = target.accept().await;
        });
        let (http_port, socks_port) = unused_port_pair().await;
        let logs = Arc::new(NetworkLogBuffer::new());
        let proxy = start_local_proxies(
            http_port,
            socks_port,
            None,
            "profile-socks".into(),
            logs.clone(),
        )
        .await
        .unwrap();

        let mut client = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut greeting = [0u8; 2];
        client.read_exact(&mut greeting).await.unwrap();
        let mut request = vec![0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1];
        request.extend_from_slice(&target_addr.port().to_be_bytes());
        client.write_all(&request).await.unwrap();
        let mut response = [0u8; 10];
        client.read_exact(&mut response).await.unwrap();

        let entry = wait_for_log(&logs).await;
        assert_eq!(entry.profile_id, "profile-socks");
        assert_eq!(entry.protocol, "socks");
        assert_eq!(entry.target, target_addr.to_string());
        assert!(entry.ok);
        assert!(entry.error.is_none());
        proxy.stop();
    }

    #[tokio::test]
    async fn socks_connect_emits_failure_log() {
        let (http_port, socks_port) = unused_port_pair().await;
        let logs = Arc::new(NetworkLogBuffer::new());
        let proxy = start_local_proxies(
            http_port,
            socks_port,
            None,
            "profile-socks-fail".into(),
            logs.clone(),
        )
        .await
        .unwrap();
        let target_port = unused_port().await;

        let mut client = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut greeting = [0u8; 2];
        client.read_exact(&mut greeting).await.unwrap();
        let mut request = vec![0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1];
        request.extend_from_slice(&target_port.to_be_bytes());
        client.write_all(&request).await.unwrap();
        let mut response = [0u8; 10];
        client.read_exact(&mut response).await.unwrap();

        let entry = wait_for_log(&logs).await;
        assert_eq!(entry.protocol, "socks");
        assert_eq!(entry.target, format!("127.0.0.1:{target_port}"));
        assert!(!entry.ok);
        assert!(entry.error.is_some());
        proxy.stop();
    }
}
