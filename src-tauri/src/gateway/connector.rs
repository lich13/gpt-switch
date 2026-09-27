use super::model::Proxy;
use hyper::Uri;
use hyper_util::{
    client::legacy::connect::{Connected, Connection},
    rt::TokioIo,
};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{
    rustls::{self, pki_types::ServerName},
    TlsConnector,
};
use tower_service::Service;
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}
pub struct Transport(TokioIo<Box<dyn Stream>>);
impl hyper::rt::Read for Transport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}
impl hyper::rt::Write for Transport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
impl Connection for Transport {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectError {
    ProxyConnect,
    ProxyAuth,
    ProxyProtocol,
    TargetConnect,
    Tls,
    Loop,
}
impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ProxyConnect => "代理连接失败或超时",
            Self::ProxyAuth => "SOCKS5 认证失败",
            Self::ProxyProtocol => "SOCKS5 握手失败",
            Self::TargetConnect => "上游目标连接失败或超时",
            Self::Tls => "上游 TLS 验证失败",
            Self::Loop => "上游指向本网关",
        })
    }
}
impl std::error::Error for ConnectError {}
impl ConnectError {
    pub fn is_proxy(self) -> bool {
        matches!(
            self,
            Self::ProxyConnect | Self::ProxyAuth | Self::ProxyProtocol
        )
    }
}
pub fn classify(error: &(dyn std::error::Error + 'static)) -> Option<ConnectError> {
    let mut current = Some(error);
    while let Some(e) = current {
        if let Some(e) = e.downcast_ref::<ConnectError>() {
            return Some(*e);
        }
        current = e.source();
    }
    None
}
#[derive(Clone)]
pub struct Connector {
    pub proxy: Option<Proxy>,
    pub timeout: Duration,
    pub gateway_port: u16,
    tls: TlsConnector,
}
impl Connector {
    pub fn new(proxy: Option<Proxy>, timeout: Duration, gateway_port: u16) -> Self {
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self {
            proxy,
            timeout,
            gateway_port,
            tls: TlsConnector::from(Arc::new(tls)),
        }
    }
    pub async fn connect(&self, uri: Uri) -> Result<Transport, ConnectError> {
        let host = uri
            .host()
            .ok_or(ConnectError::TargetConnect)?
            .trim_matches(['[', ']']);
        let tls = uri.scheme_str() == Some("https");
        let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
        let tcp = if let Some(proxy) = &self.proxy {
            let mut tcp = tokio::time::timeout(
                self.timeout,
                TcpStream::connect((proxy.host.as_str(), proxy.port)),
            )
            .await
            .map_err(|_| ConnectError::ProxyConnect)?
            .map_err(|_| ConnectError::ProxyConnect)?;
            tokio::time::timeout(self.timeout, authenticate(&mut tcp, proxy))
                .await
                .map_err(|_| ConnectError::ProxyConnect)??;
            tokio::time::timeout(self.timeout, socks_target(&mut tcp, host, port))
                .await
                .map_err(|_| ConnectError::TargetConnect)??;
            tcp
        } else {
            tokio::time::timeout(self.timeout, async {
                let addresses = tokio::net::lookup_host((host, port))
                    .await
                    .map_err(|_| ConnectError::TargetConnect)?;
                let mut stream = None;
                for address in addresses {
                    if port == self.gateway_port && address.ip().is_loopback() {
                        return Err(ConnectError::Loop);
                    }
                    if let Ok(s) = TcpStream::connect(address).await {
                        stream = Some(s);
                        break;
                    }
                }
                stream.ok_or(ConnectError::TargetConnect)
            })
            .await
            .map_err(|_| ConnectError::TargetConnect)??
        };
        let _ = tcp.set_nodelay(true);
        let stream: Box<dyn Stream> = if tls {
            let server = ServerName::try_from(host.to_owned()).map_err(|_| ConnectError::Tls)?;
            Box::new(
                tokio::time::timeout(self.timeout, self.tls.connect(server, tcp))
                    .await
                    .map_err(|_| ConnectError::Tls)?
                    .map_err(|_| ConnectError::Tls)?,
            )
        } else {
            Box::new(tcp)
        };
        Ok(Transport(TokioIo::new(stream)))
    }
}
async fn authenticate(tcp: &mut TcpStream, proxy: &Proxy) -> Result<(), ConnectError> {
    let method = if proxy.username.is_empty() { 0 } else { 2 };
    tcp.write_all(&[5, 1, method])
        .await
        .map_err(|_| ConnectError::ProxyProtocol)?;
    let mut reply = [0; 2];
    tcp.read_exact(&mut reply)
        .await
        .map_err(|_| ConnectError::ProxyProtocol)?;
    if reply != [5, method] {
        return Err(ConnectError::ProxyAuth);
    }
    if method == 2 {
        let mut auth = vec![1, proxy.username.len() as u8];
        auth.extend_from_slice(proxy.username.as_bytes());
        auth.push(proxy.password.len() as u8);
        auth.extend_from_slice(proxy.password.as_bytes());
        tcp.write_all(&auth)
            .await
            .map_err(|_| ConnectError::ProxyAuth)?;
        tcp.read_exact(&mut reply)
            .await
            .map_err(|_| ConnectError::ProxyAuth)?;
        if reply != [1, 0] {
            return Err(ConnectError::ProxyAuth);
        }
    }
    Ok(())
}
async fn socks_target(tcp: &mut TcpStream, host: &str, port: u16) -> Result<(), ConnectError> {
    let mut request = vec![5, 1, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            if host.len() > 255 {
                return Err(ConnectError::TargetConnect);
            }
            request.extend_from_slice(&[3, host.len() as u8]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    tcp.write_all(&request)
        .await
        .map_err(|_| ConnectError::TargetConnect)?;
    let mut reply = [0; 4];
    tcp.read_exact(&mut reply)
        .await
        .map_err(|_| ConnectError::TargetConnect)?;
    if reply[0] != 5 || reply[2] != 0 {
        return Err(ConnectError::ProxyProtocol);
    }
    if reply[1] != 0 {
        return Err(ConnectError::TargetConnect);
    }
    let length = match reply[3] {
        1 => 4,
        4 => 16,
        3 => tcp
            .read_u8()
            .await
            .map_err(|_| ConnectError::ProxyProtocol)? as usize,
        _ => return Err(ConnectError::ProxyProtocol),
    };
    let mut rest = vec![0; length + 2];
    tcp.read_exact(&mut rest)
        .await
        .map_err(|_| ConnectError::ProxyProtocol)?;
    Ok(())
}
impl Service<Uri> for Connector {
    type Response = Transport;
    type Error = ConnectError;
    type Future = Pin<Box<dyn Future<Output = Result<Transport, ConnectError>> + Send>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, uri: Uri) -> Self::Future {
        let this = self.clone();
        Box::pin(async move { this.connect(uri).await })
    }
}
