//! These codes are copied from `tonic/src/codec/compression.rs` and may be modified by us.

use std::io;

use bytes::{Buf, BufMut, BytesMut};
pub use flate2::Compression as Level;
use flate2::bufread::{GzDecoder, GzEncoder, ZlibDecoder, ZlibEncoder};
use http::HeaderValue;
use pilota::LinkedBytes;

use super::BUFFER_SIZE;
use crate::Status;

pub const ENCODING_HEADER: &str = "grpc-encoding";
pub const ACCEPT_ENCODING_HEADER: &str = "grpc-accept-encoding";
const DEFAULT_LEVEL: Level = Level::new(6);

/// The compression encodings volo supports.
#[derive(Clone, Copy, Debug)]
pub enum CompressionEncoding {
    Identity,
    Gzip(Option<GzipConfig>),
    Zlib(Option<ZlibConfig>),
    Zstd(Option<ZstdConfig>),
}

impl PartialEq for CompressionEncoding {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Gzip(_), Self::Gzip(_))
                | (Self::Zlib(_), Self::Zlib(_))
                | (Self::Zstd(_), Self::Zstd(_))
                | (Self::Identity, Self::Identity)
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GzipConfig {
    pub level: Level,
}

impl Default for GzipConfig {
    fn default() -> Self {
        Self {
            level: DEFAULT_LEVEL,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ZlibConfig {
    pub level: Level,
}

impl Default for ZlibConfig {
    fn default() -> Self {
        Self {
            level: DEFAULT_LEVEL,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ZstdConfig {
    pub level: Level,
}

impl Default for ZstdConfig {
    fn default() -> Self {
        Self {
            level: DEFAULT_LEVEL,
        }
    }
}

/// compose multiple compression encodings to a [HeaderValue]
pub fn compose_encodings(encodings: &[CompressionEncoding]) -> HeaderValue {
    let encodings = encodings
        .iter()
        .map(|item| match item {
            // TODO: gzip-6 @https://grpc.github.io/grpc/core/md_doc_compression.html#autotoc_md59
            CompressionEncoding::Gzip(_) => "gzip",
            CompressionEncoding::Zlib(_) => "zlib",
            CompressionEncoding::Zstd(_) => "zstd",
            CompressionEncoding::Identity => "identity",
        })
        .collect::<Vec<&'static str>>();
    // encodings.push("identity");

    HeaderValue::from_str(encodings.join(",").as_str()).unwrap()
}

fn is_enabled(encoding: CompressionEncoding, encodings: &[CompressionEncoding]) -> bool {
    encodings.contains(&encoding)
}

impl CompressionEncoding {
    /// make the compression encoding into a [HeaderValue]
    pub fn into_header_value(self) -> HeaderValue {
        match self {
            CompressionEncoding::Gzip(_) => HeaderValue::from_static("gzip"),
            CompressionEncoding::Zlib(_) => HeaderValue::from_static("zlib"),
            CompressionEncoding::Zstd(_) => HeaderValue::from_static("zstd"),
            CompressionEncoding::Identity => HeaderValue::from_static("identity"),
        }
    }

    /// make the compression encodings into a [HeaderValue],and the encodings uses a `,` as
    /// separator
    pub fn into_accept_encoding_header_value(
        self,
        encodings: &[CompressionEncoding],
    ) -> Option<HeaderValue> {
        if self.is_enabled() {
            Some(compose_encodings(encodings))
        } else {
            None
        }
    }

    /// Based on the `grpc-accept-encoding` header, adaptive picking an encoding to use.
    pub fn from_accept_encoding_header(
        headers: &http::HeaderMap,
        config: &Option<Vec<Self>>,
    ) -> Option<Self> {
        if let Some(available_encodings) = config {
            let header_value = headers.get(ACCEPT_ENCODING_HEADER)?;
            let header_value_str = header_value.to_str().ok()?;

            header_value_str
                .split(',')
                .map(|s| s.trim())
                .find_map(|encoding| match encoding {
                    "gzip" => available_encodings.iter().find_map(|item| {
                        if item.is_gzip_enabled() {
                            Some(*item)
                        } else {
                            None
                        }
                    }),
                    "zlib" => available_encodings.iter().find_map(|item| {
                        if item.is_zlib_enabled() {
                            Some(*item)
                        } else {
                            None
                        }
                    }),
                    "zstd" => available_encodings.iter().find_map(|item| {
                        if item.is_zstd_enabled() {
                            Some(*item)
                        } else {
                            None
                        }
                    }),
                    _ => None,
                })
        } else {
            None
        }
    }

    /// Get the value of `grpc-encoding` header. Returns an error if the encoding isn't supported.
    #[allow(clippy::result_large_err)]
    pub fn from_encoding_header(
        headers: &http::HeaderMap,
        config: &Option<Vec<Self>>,
    ) -> Result<Option<Self>, Status> {
        if let Some(encodings) = config {
            let header_value = if let Some(header_value) = headers.get(ENCODING_HEADER) {
                header_value
            } else {
                return Ok(None);
            };

            match header_value.to_str()? {
                "gzip" if is_enabled(Self::Gzip(None), encodings) => Ok(Some(Self::Gzip(None))),
                "zlib" if is_enabled(Self::Zlib(None), encodings) => Ok(Some(Self::Zlib(None))),
                "zstd" if is_enabled(Self::Zstd(None), encodings) => Ok(Some(Self::Zstd(None))),
                "identity" => Ok(None),
                other => {
                    let status = Status::unimplemented(format!(
                        "Content is compressed with `{other}` which isn't supported"
                    ));
                    Err(status)
                }
            }
        } else {
            Ok(None)
        }
    }

    /// please use it only for Compression type is insignificant, otherwise you will have a
    /// duplicate pattern-matching problem
    pub fn level(self) -> Level {
        match self {
            CompressionEncoding::Gzip(Some(config)) => config.level,
            CompressionEncoding::Zlib(Some(config)) => config.level,
            CompressionEncoding::Zstd(Some(config)) => config.level,
            _ => DEFAULT_LEVEL,
        }
    }

    const fn is_gzip_enabled(&self) -> bool {
        matches!(self, CompressionEncoding::Gzip(_))
    }

    const fn is_zlib_enabled(&self) -> bool {
        matches!(self, CompressionEncoding::Zlib(_))
    }

    const fn is_zstd_enabled(&self) -> bool {
        matches!(self, CompressionEncoding::Zstd(_))
    }

    const fn is_enabled(&self) -> bool {
        matches!(
            self,
            CompressionEncoding::Gzip(_)
                | CompressionEncoding::Zlib(_)
                | CompressionEncoding::Zstd(_)
        )
    }
}

/// Compress `len` bytes from `src_buf` into `dest_buf`.
pub(crate) fn compress(
    encoding: CompressionEncoding,
    src_buf: &mut LinkedBytes,
    dest_buf: &mut LinkedBytes,
) -> Result<(), io::Error> {
    let len = src_buf.bytes().len();
    let capacity = ((len / BUFFER_SIZE) + 1) * BUFFER_SIZE;

    dest_buf.reserve(capacity);

    match encoding {
        CompressionEncoding::Gzip(Some(config)) => {
            let mut gz_encoder = GzEncoder::new(&src_buf.bytes()[0..len], config.level);
            io::copy(&mut gz_encoder, &mut dest_buf.writer())?;
        }
        CompressionEncoding::Zlib(Some(config)) => {
            let mut zlib_encoder = ZlibEncoder::new(&src_buf.bytes()[0..len], config.level);
            io::copy(&mut zlib_encoder, &mut dest_buf.writer())?;
        }
        CompressionEncoding::Zstd(Some(config)) => {
            let level = config.level.level();
            let zstd_level = if level == 0 {
                zstd::DEFAULT_COMPRESSION_LEVEL
            } else {
                level as i32
            };
            let mut zstd_encoder = zstd::Encoder::new(dest_buf.writer(), zstd_level)?;
            io::copy(&mut &src_buf.bytes()[0..len], &mut zstd_encoder)?;
            zstd_encoder.finish()?;
        }
        _ => {}
    };

    src_buf.bytes_mut().advance(len);
    Ok(())
}

/// Decompress `len` bytes from `src_buf` into `dest_buf`.
pub(crate) fn decompress(
    encoding: CompressionEncoding,
    src_buf: &mut BytesMut,
    dest_buf: &mut BytesMut,
) -> Result<(), io::Error> {
    let len = src_buf.len();
    let estimate_decompressed_len = len * 2;
    let capacity = ((estimate_decompressed_len / BUFFER_SIZE) + 1) * BUFFER_SIZE;

    dest_buf.reserve(capacity);

    match encoding {
        CompressionEncoding::Gzip(_) => {
            let mut gz_decoder = GzDecoder::new(&src_buf[0..len]);
            io::copy(&mut gz_decoder, &mut dest_buf.writer())?;
        }

        CompressionEncoding::Zlib(_) => {
            let mut zlib_decoder = ZlibDecoder::new(&src_buf[0..len]);
            io::copy(&mut zlib_decoder, &mut dest_buf.writer())?;
        }
        CompressionEncoding::Zstd(_) => {
            let mut zstd_decoder = zstd::Decoder::new(&src_buf[0..len])?;
            io::copy(&mut zstd_decoder, &mut dest_buf.writer())?;
        }
        _ => {}
    };

    src_buf.advance(len);
    Ok(())
}

#[cfg(test)]
mod tests {
    use bytes::BufMut;
    use pilota::LinkedBytes;

    use crate::codec::{
        BUFFER_SIZE,
        compression::{
            CompressionEncoding, GzipConfig, Level, ZlibConfig, ZstdConfig, compress, decompress,
        },
    };

    #[test]
    fn test_consistency_for_compression() {
        let mut src = LinkedBytes::with_capacity(BUFFER_SIZE);
        let mut compress_buf = LinkedBytes::new();
        let mut de_data = LinkedBytes::with_capacity(BUFFER_SIZE);
        let test_data = &b"test compression"[..];
        src.put(test_data);

        let encodings = [
            CompressionEncoding::Gzip(Some(GzipConfig {
                level: Level::fast(),
            })),
            CompressionEncoding::Zlib(Some(ZlibConfig {
                level: Level::fast(),
            })),
            CompressionEncoding::Zstd(Some(ZstdConfig {
                level: Level::new(3),
            })),
            CompressionEncoding::Identity,
        ];

        for encoding in encodings {
            compress_buf.reset();
            compress(encoding, &mut src, &mut compress_buf).expect("compress failed:");
            decompress(encoding, compress_buf.bytes_mut(), de_data.bytes_mut())
                .expect("decompress failed:");
            assert_eq!(test_data, de_data.bytes());
        }
    }
}
