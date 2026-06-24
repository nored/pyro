//! Source image readers: transparent decompression with byte accounting.

use std::fs::File;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Wraps a reader and counts the bytes pulled through it.
pub struct CountingReader<R> {
    inner: R,
    counter: Arc<AtomicU64>,
}

impl<R: Read> CountingReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            counter: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn counter(&self) -> Arc<AtomicU64> {
        self.counter.clone()
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.counter.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Open the raw source bytes for an image path, which may be a local file or an
/// `http(s)://` URL (streamed). Returns a reader the caller wraps for
/// decompression.
pub fn open_raw(image_path: &str) -> io::Result<Box<dyn Read + Send>> {
    if image_path.starts_with("http://") || image_path.starts_with("https://") {
        let resp = ureq::get(image_path)
            .call()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("download failed: {e}")))?;
        Ok(Box::new(resp.into_reader()))
    } else {
        Ok(Box::new(File::open(image_path)?))
    }
}

/// Build a streaming decoder for the non-zip formats over an arbitrary source
/// reader (file or network). Returns the decoded reader plus a counter tracking
/// *source* bytes consumed, a good progress numerator against the total size.
pub fn streaming_decoder(
    raw: Box<dyn Read + Send>,
    compression: &str,
) -> io::Result<(Box<dyn Read>, Arc<AtomicU64>)> {
    let counted = CountingReader::new(raw);
    let counter = counted.counter();
    let reader: Box<dyn Read> = match compression {
        "none" => Box::new(counted),
        "gzip" => Box::new(flate2::read::MultiGzDecoder::new(counted)),
        "xz" => Box::new(xz2::read::XzDecoder::new(counted)),
        "zstd" => Box::new(zstd::stream::read::Decoder::new(counted)?),
        "bzip2" => Box::new(bzip2::read::BzDecoder::new(counted)),
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported compression: {other}"),
            ))
        }
    };
    Ok((reader, counter))
}
