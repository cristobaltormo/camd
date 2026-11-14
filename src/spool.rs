use std::fs::File;
use std::io::{BufReader, BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

pub const HEADER_LEN: usize = 12;
pub use crate::grabber::MAX_FRAME_BYTES;

pub struct Spool {
    pub path: PathBuf,
    pub count: usize,
    pub first_ts: f64,
    pub last_ts: f64,
    file: Option<BufWriter<File>>,
}

impl Spool {
    pub fn create(path: &Path) -> std::io::Result<Spool> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(path)?;
        Ok(Spool {
            path: path.to_path_buf(),
            count: 0,
            first_ts: 0.0,
            last_ts: 0.0,
            file: Some(BufWriter::new(file)),
        })
    }

    pub fn load(path: &Path) -> std::io::Result<Spool> {
        let mut spool = Spool {
            path: path.to_path_buf(),
            count: 0,
            first_ts: 0.0,
            last_ts: 0.0,
            file: None,
        };
        for (ts, _) in spool.frames()? {
            spool.note(ts);
        }
        Ok(spool)
    }

    fn note(&mut self, ts: f64) {
        if self.first_ts == 0.0 {
            self.first_ts = ts;
        }
        self.last_ts = ts;
        self.count += 1;
    }

    pub fn duration(&self) -> f64 {
        (self.last_ts - self.first_ts).max(0.1)
    }

    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    pub fn append(&mut self, ts: f64, jpeg: &[u8]) -> std::io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("spool is closed"))?;
        if jpeg.len() > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "frame too large",
            ));
        }
        let length = u32::try_from(jpeg.len())
            .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "frame too large"))?;
        file.write_all(&ts.to_be_bytes())?;
        file.write_all(&length.to_be_bytes())?;
        file.write_all(jpeg)?;
        self.note(ts);
        Ok(())
    }

    pub fn close(&mut self) -> std::io::Result<()> {
        match self.file.take() {
            Some(mut file) => file.flush(),
            None => Ok(()),
        }
    }

    pub fn frames(&self) -> std::io::Result<Frames> {
        Ok(Frames {
            reader: BufReader::new(File::open(&self.path)?),
        })
    }

    pub fn delete(&mut self) -> std::io::Result<()> {
        let _ = self.close();
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

pub struct Frames {
    reader: BufReader<File>,
}

fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> bool {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => return false,
            Ok(n) => filled += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
    true
}

impl Iterator for Frames {
    type Item = (f64, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        let mut header = [0u8; HEADER_LEN];
        if !read_full(&mut self.reader, &mut header) {
            return None;
        }
        let ts = f64::from_be_bytes(header[..8].try_into().ok()?);
        let length = u32::from_be_bytes(header[8..].try_into().ok()?) as usize;
        if length > MAX_FRAME_BYTES {
            return None;
        }
        let mut jpeg = vec![0u8; length];
        if !read_full(&mut self.reader, &mut jpeg) {
            return None;
        }
        Some((ts, jpeg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_roundtrip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut spool = Spool::create(&dir.path().join("x.spool")).unwrap();
        for i in 0..5u8 {
            spool
                .append(100.0 + i as f64, &vec![i; 10 + i as usize])
                .unwrap();
        }
        spool.close().unwrap();
        assert_eq!(
            (spool.count, spool.first_ts, spool.last_ts),
            (5, 100.0, 104.0)
        );
        let got: Vec<(f64, usize)> = spool
            .frames()
            .unwrap()
            .map(|(ts, j)| (ts, j.len()))
            .collect();
        assert_eq!(
            got,
            (0..5)
                .map(|i| (100.0 + i as f64, 10 + i))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_truncated_spool_loads_what_is_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.spool");
        let mut spool = Spool::create(&path).unwrap();
        for i in 0..3 {
            spool.append((i + 1) as f64, &[b'a'; 50]).unwrap();
        }
        spool.close().unwrap();
        let data = std::fs::read(&path).unwrap();
        std::fs::write(&path, &data[..data.len() - 20]).unwrap();
        let loaded = Spool::load(&path).unwrap();
        assert_eq!((loaded.count, loaded.last_ts), (2, 2.0));
    }

    #[test]
    fn a_corrupt_length_ends_the_spool_instead_of_allocating_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.spool");
        let mut spool = Spool::create(&path).unwrap();
        spool.append(1.0, b"jpg").unwrap();
        spool.close().unwrap();
        let mut data = std::fs::read(&path).unwrap();
        data.extend_from_slice(&2.0f64.to_be_bytes());
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        data.extend_from_slice(&[0u8; 64]);
        std::fs::write(&path, &data).unwrap();
        let loaded = Spool::load(&path).unwrap();
        assert_eq!((loaded.count, loaded.last_ts), (1, 1.0));
        let mut fresh = Spool::create(&dir.path().join("y.spool")).unwrap();
        assert!(fresh.append(1.0, &vec![0u8; MAX_FRAME_BYTES + 1]).is_err());
    }

    #[test]
    fn delete_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.spool");
        let mut spool = Spool::create(&path).unwrap();
        spool.append(1.0, b"z").unwrap();
        spool.delete().unwrap();
        assert!(!path.exists());
        spool.delete().unwrap();
    }

    #[test]
    fn the_layout_is_big_endian_double_and_u32() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.spool");
        let mut spool = Spool::create(&path).unwrap();
        spool.append(1.5, b"jpg").unwrap();
        spool.close().unwrap();
        let mut expected = 1.5f64.to_be_bytes().to_vec();
        expected.extend_from_slice(&3u32.to_be_bytes());
        expected.extend_from_slice(b"jpg");
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        assert!(spool.append(2.0, b"x").is_err());
    }

    #[test]
    fn duration_has_a_floor() {
        let dir = tempfile::tempdir().unwrap();
        let mut spool = Spool::create(&dir.path().join("x.spool")).unwrap();
        spool.append(5.0, b"a").unwrap();
        assert_eq!(spool.duration(), 0.1);
    }
}
