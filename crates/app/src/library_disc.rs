use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

use image::banner::Banner;
use image::dvd::{DVD_HEADER_SIZE, Header};
use image::rvz;
use indexed_deflate::{AccessPointSpan, DeflateIndexBuilder};
use zerocopy::FromBytes;

pub fn read_metadata(path: &Path) -> anyhow::Result<(Header, Option<Banner>)> {
    self::read_stream(self::open(path)?)
}

pub fn read_header(path: &Path) -> anyhow::Result<Header> {
    self::header_from_stream(self::open(path)?)
}

fn open(path: &Path) -> io::Result<ScanStream> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    Ok(ScanStream::new(BufReader::new(file), len))
}

fn header_from_stream(stream: ScanStream) -> anyhow::Result<Header> {
    let mut stream = self::unwrap_zip(stream)?;
    let mut bytes = [0; DVD_HEADER_SIZE];
    stream.read_exact(&mut bytes)?;

    if bytes.starts_with(&rvz::RVZ_MAGIC) {
        bytes.copy_within(
            rvz::DISC_HEADER_OFFSET..rvz::DISC_HEADER_OFFSET + rvz::DISC_HEADER_SIZE,
            0,
        );
        bytes[rvz::DISC_HEADER_SIZE..].fill(0);
    }

    let header = Header::read_from_bytes(&bytes).unwrap();
    anyhow::ensure!(header.is_gc() || header.is_wii(), "invalid disc header");
    Ok(header)
}

fn unwrap_zip(mut stream: ScanStream) -> anyhow::Result<ScanStream> {
    let mut magic = [0; 4];
    stream.read_exact(&mut magic)?;
    stream.rewind()?;

    if &magic == b"PK\x03\x04" {
        return self::open_zip(stream);
    }
    Ok(stream)
}

fn read_stream(stream: ScanStream) -> anyhow::Result<(Header, Option<Banner>)> {
    let stream = self::unwrap_zip(stream)?;
    let disc = nod::Disc::new_stream(Box::new(stream))?;
    let mut partition = disc.open_partition_kind(nod::PartitionKind::Data)?;

    let mut bytes = [0; DVD_HEADER_SIZE];
    partition.read_exact(&mut bytes)?;
    if disc.header().is_wii() {
        image::dvd::unshift_wii_header_offsets(&mut bytes);
    }
    let header = Header::read_from_bytes(&bytes).unwrap();

    let banner = image::banner::extract(
        &header,
        |offset, buf| {
            partition.seek(SeekFrom::Start(offset as u64))?;
            partition.read_exact(buf)
        },
        crate::banner::decode,
    )?;
    Ok((header, banner))
}

fn open_zip(stream: ScanStream) -> anyhow::Result<ScanStream> {
    let file_len = stream.len;
    let mut archive = zip::ZipArchive::new(stream)?;
    let index = archive
        .file_names()
        .position(image::is_disc_entry)
        .ok_or_else(|| anyhow::anyhow!("no disc image found in ZIP"))?;

    let entry = archive.by_index_raw(index)?;
    anyhow::ensure!(!entry.encrypted(), "encrypted ZIP disc images are unsupported");
    let start = entry
        .data_start()
        .ok_or_else(|| anyhow::anyhow!("missing ZIP data offset"))?;
    let compressed = entry.compressed_size();
    let len = entry.size();
    let method = entry.compression();
    anyhow::ensure!(
        start <= file_len && compressed <= file_len - start,
        "truncated ZIP entry"
    );
    drop(entry);

    let range = nod::WindowedStream::new(BufReader::new(archive.into_inner()), start, compressed)?;
    match method {
        zip::CompressionMethod::Stored => {
            anyhow::ensure!(compressed == len, "invalid stored ZIP entry size");
            Ok(ScanStream::new(range, len))
        }
        zip::CompressionMethod::Deflated => {
            let span = AccessPointSpan::new(compressed.div_ceil(64).max(1024 * 1024));
            let reader = DeflateIndexBuilder::new(range, Cursor::new(Vec::new()), span)?;
            Ok(ScanStream::new(reader, len))
        }
        _ => anyhow::bail!("unsupported ZIP compression method: {method}"),
    }
}

trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

#[derive(Clone)]
struct ScanStream {
    reader: Arc<Mutex<SharedReader>>,
    position: u64,
    len: u64,
}

struct SharedReader {
    reader: Box<dyn ReadSeek>,
    position: Option<u64>,
}

impl ScanStream {
    fn new(reader: impl Read + Seek + Send + 'static, len: u64) -> Self {
        Self {
            reader: Arc::new(Mutex::new(SharedReader {
                reader: Box::new(reader),
                position: None,
            })),
            position: 0,
            len,
        }
    }
}

impl Read for ScanStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.len.saturating_sub(self.position).min(buf.len() as u64) as usize;
        if len == 0 {
            return Ok(0);
        }

        let mut reader = self
            .reader
            .lock()
            .map_err(|_| io::Error::other("disc reader poisoned"))?;
        let in_place = reader.position == Some(self.position);
        reader.position = None;
        if !in_place {
            reader.reader.seek(SeekFrom::Start(self.position))?;
        }

        let read = reader.reader.read(&mut buf[..len])?;
        self.position += read as u64;
        reader.position = Some(self.position);
        Ok(read)
    }
}

impl Seek for ScanStream {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.len.checked_add_signed(offset),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid disc seek"))?;

        self.position = position;
        Ok(position)
    }
}
