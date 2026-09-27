use super::connector::BoxError;
use crate::storage;
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full, StreamBody};
use hyper::{
    body::{Body, Frame},
    HeaderMap,
};
use std::{io, path::Path, sync::Arc};
use tokio::io::AsyncWriteExt;
pub type WireBody = UnsyncBoxBody<Bytes, BoxError>;
const MEMORY_LIMIT: usize = 2 * 1024 * 1024;
const MAX_BODY: u64 = 1024 * 1024 * 1024;
pub fn empty() -> WireBody {
    Full::new(Bytes::new())
        .map_err(|e| match e {})
        .boxed_unsync()
}
pub fn full(bytes: impl Into<Bytes>) -> WireBody {
    Full::new(bytes.into())
        .map_err(|e| match e {})
        .boxed_unsync()
}
enum Payload {
    Memory(Bytes),
    Disk(Arc<tempfile::NamedTempFile>),
}
#[derive(serde::Deserialize, Default)]
pub struct RequestHints {
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub stream: bool,
}
pub struct Replay {
    payload: Payload,
    trailers: Option<HeaderMap>,
    pub length: u64,
}
impl Replay {
    pub async fn capture<B>(mut body: B, dir: &Path) -> Result<Self, BoxError>
    where
        B: Body<Data = Bytes> + Unpin,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut memory = BytesMut::new();
        let mut disk = None;
        let mut writer = None;
        let mut length = 0;
        let mut trailers = None;
        while let Some(frame) = body.frame().await {
            let frame = frame?;
            match frame.into_data() {
                Ok(chunk) => {
                    length += chunk.len() as u64;
                    if length > MAX_BODY {
                        return Err(io::Error::other("请求超过 1 GiB").into());
                    }
                    if writer.is_none() && memory.len() + chunk.len() > MEMORY_LIMIT {
                        let f = tempfile::NamedTempFile::new_in(dir)?;
                        storage::protect(f.path(), false)?;
                        let mut file = tokio::fs::File::from_std(f.reopen()?);
                        file.write_all(&memory).await?;
                        memory.clear();
                        writer = Some(file);
                        disk = Some(Arc::new(f));
                    }
                    if let Some(f) = writer.as_mut() {
                        f.write_all(&chunk).await?;
                    } else {
                        memory.extend_from_slice(&chunk);
                    }
                }
                Err(frame) => {
                    if let Ok(t) = frame.into_trailers() {
                        trailers = Some(t);
                    }
                }
            }
        }
        if let Some(f) = writer.as_mut() {
            f.flush().await?;
        }
        Ok(Self {
            payload: disk
                .map(Payload::Disk)
                .unwrap_or_else(|| Payload::Memory(memory.freeze())),
            trailers,
            length,
        })
    }
    pub fn body(&self) -> WireBody {
        let trailers = self.trailers.clone();
        let body = match &self.payload {
            Payload::Memory(bytes) => {
                let bytes = bytes.clone();
                StreamBody::new(
                    async_stream::try_stream! {if !bytes.is_empty(){yield Frame::data(bytes);}
                    if let Some(t)=trailers{yield Frame::trailers(t);}},
                )
                .boxed_unsync()
            }
            Payload::Disk(file) => {
                let file = file.clone();
                StreamBody::new(async_stream::try_stream! {
                    let reader=tokio::fs::File::open(file.path()).await?;
                    let mut stream=tokio_util::io::ReaderStream::with_capacity(reader,64*1024);
                    while let Some(bytes)=stream.next().await{yield Frame::data(bytes?);}
                    if let Some(t)=trailers{yield Frame::trailers(t);}
                    drop(file);
                })
                .boxed_unsync()
            }
        };
        body
    }
    pub async fn hints(&self, encoding: &str) -> Result<RequestHints, BoxError> {
        let reader: Box<dyn std::io::Read + Send> = match &self.payload {
            Payload::Memory(bytes) => Box::new(std::io::Cursor::new(bytes.clone())),
            Payload::Disk(file) => Box::new(file.reopen()?),
        };
        let encoding = encoding.to_owned();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let decoded: Box<dyn Read> = match encoding.as_str() {
                "identity" => reader,
                "gzip" => Box::new(flate2::read::GzDecoder::new(reader)),
                "deflate" => Box::new(flate2::read::ZlibDecoder::new(reader)),
                "zstd" => Box::new(zstd::stream::read::Decoder::new(reader)?),
                _ => return Err(io::Error::other("unsupported inspection encoding").into()),
            };
            // serde ignores unknown fields while streaming, including large input arrays.
            let hints: RequestHints = serde_json::from_reader(decoded.take(MAX_BODY + 1))?;
            if hints
                .previous_response_id
                .as_ref()
                .is_some_and(|id| id.len() > 1024)
            {
                return Err(io::Error::other("invalid response cursor").into());
            }
            Ok(hints)
        })
        .await
        .map_err(|_| io::Error::other("request inspection interrupted"))?
    }
    pub fn has_trailers(&self) -> bool {
        self.trailers.is_some()
    }
}
