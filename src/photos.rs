use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::config::SharedConfig;
use crate::db::{Database, PhotoRow};
use crate::error::Error;
use crate::storage::unlink;

pub const MAX_PHOTOS: i64 = 1000;
pub const THUMB_WIDTH: u32 = 360;
pub const THUMB_QUALITY: u8 = 80;

#[derive(Debug, Clone, PartialEq)]
pub enum PhotoError {
    NoFrame,
    Limit(String),
    Undecodable(String),
    Internal(String),
}

impl PhotoError {
    pub fn status(&self) -> u16 {
        match self {
            PhotoError::NoFrame => 503,
            PhotoError::Limit(_) => 409,
            PhotoError::Undecodable(_) => 422,
            PhotoError::Internal(_) => 500,
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            PhotoError::NoFrame => "no frame",
            PhotoError::Limit(m) | PhotoError::Undecodable(m) | PhotoError::Internal(m) => m,
        }
    }
}

impl From<Error> for PhotoError {
    fn from(e: Error) -> Self {
        PhotoError::Internal(e.to_string())
    }
}

impl From<std::io::Error> for PhotoError {
    fn from(e: std::io::Error) -> Self {
        PhotoError::Internal(e.to_string())
    }
}

pub fn thumbnail(jpeg: &[u8]) -> Result<Vec<u8>, PhotoError> {
    let image = crate::imaging::decode_rgb(jpeg)
        .ok_or_else(|| PhotoError::Undecodable("not a decodable jpeg".to_string()))?;
    let width = THUMB_WIDTH as usize;
    let height = ((image.height as u64 * width as u64) / image.width as u64).max(1) as usize;
    let small = crate::imaging::resize_area(&image, width, height);
    crate::imaging::encode_jpeg(&small, THUMB_QUALITY)
        .ok_or_else(|| PhotoError::Internal("thumbnail encode failed".to_string()))
}

pub struct Photos {
    cfg: SharedConfig,
    db: Arc<Database>,
    pub max_photos: i64,
}

impl Photos {
    pub fn new(cfg: SharedConfig, db: Arc<Database>) -> Photos {
        Photos {
            cfg,
            db,
            max_photos: MAX_PHOTOS,
        }
    }

    fn photos_dir(&self) -> PathBuf {
        self.cfg
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .photos_dir()
    }

    pub fn usage(&self) -> (i64, i64) {
        self.db.photos_usage().unwrap_or((0, 0))
    }

    pub fn save(&self, jpeg: &[u8], ts: f64) -> Result<Value, PhotoError> {
        if self.db.count_photos()? >= self.max_photos {
            return Err(PhotoError::Limit(format!(
                "photo limit of {} reached",
                self.max_photos
            )));
        }
        let small = thumbnail(jpeg)?;
        let id = self.db.insert_photo(ts)?;
        let written = (|| -> Result<(PathBuf, PathBuf, i64), PhotoError> {
            let folder = self.photos_dir().join(crate::clock::local(ts).month_path());
            std::fs::create_dir_all(&folder)?;
            let (path, thumb) = (
                folder.join(format!("{id}.jpg")),
                folder.join(format!("{id}_t.jpg")),
            );
            std::fs::write(&path, jpeg)?;
            std::fs::write(&thumb, &small)?;
            let size = (std::fs::metadata(&path)?.len() + std::fs::metadata(&thumb)?.len()) as i64;
            Ok((path, thumb, size))
        })();
        match written {
            Ok((path, thumb, size)) => {
                self.db.set_photo_files(
                    id,
                    &path.to_string_lossy(),
                    &thumb.to_string_lossy(),
                    size,
                )?;
                Ok(serde_json::json!({"id": id, "ts": ts, "size": size}))
            }
            Err(e) => {
                let _ = self.db.delete_photo_row(id);
                Err(e)
            }
        }
    }

    pub fn list(&self, limit: i64, before: Option<f64>) -> crate::error::Result<Vec<Value>> {
        Ok(self
            .db
            .list_photos(limit, before)?
            .iter()
            .map(PhotoRow::to_json)
            .collect())
    }

    pub fn get(&self, id: i64) -> Option<PhotoRow> {
        self.db.photo(id).ok().flatten()
    }

    pub fn file(&self, id: i64, thumb: bool) -> Option<PathBuf> {
        let row = self.get(id)?;
        let path = PathBuf::from(if thumb { &row.thumb_path } else { &row.path });
        inside(&path, &self.photos_dir()).then_some(path)
    }

    pub fn remove(&self, id: i64) -> bool {
        let Some(row) = self.get(id) else {
            return false;
        };
        unlink(Some(&row.path));
        unlink(Some(&row.thumb_path));
        self.db.delete_photo_row(id).is_ok()
    }
}

pub fn inside(path: &Path, root: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(file), Ok(root)) => file.starts_with(&root) && file != root,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imaging::{Image, decode_rgb, encode_jpeg};

    #[test]
    fn thumbnails_are_360_wide_and_keep_the_aspect() {
        let jpeg = encode_jpeg(&Image::filled(800, 600, 3, 120), 90).unwrap();
        let small = decode_rgb(&thumbnail(&jpeg).unwrap()).unwrap();
        assert_eq!((small.width, small.height), (360, 270));
        let tall = encode_jpeg(&Image::filled(100, 1, 3, 10), 90).unwrap();
        let wide = decode_rgb(&thumbnail(&tall).unwrap()).unwrap();
        assert_eq!((wide.width, wide.height), (360, 3));
        let strip = encode_jpeg(&Image::filled(2000, 1, 1, 10), 90).unwrap();
        assert_eq!(decode_rgb(&thumbnail(&strip).unwrap()).unwrap().height, 1);
    }

    #[test]
    fn undecodable_frames_are_rejected() {
        assert_eq!(
            thumbnail(b"nope"),
            Err(PhotoError::Undecodable("not a decodable jpeg".into()))
        );
    }

    #[test]
    fn photos_are_saved_listed_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = crate::config::Config::with_data_dir(dir.path());
        let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
        let mut photos = Photos::new(cfg.shared(), db);
        let jpeg = encode_jpeg(&Image::filled(640, 480, 3, 60), 90).unwrap();
        let saved = photos.save(&jpeg, 1_700_000_000.0).unwrap();
        let id = saved["id"].as_i64().unwrap();
        assert_eq!(photos.list(10, None).unwrap().len(), 1);
        assert!(photos.file(id, true).is_some() && photos.file(id, false).is_some());
        photos.max_photos = 1;
        assert_eq!(
            photos.save(&jpeg, 1.0),
            Err(PhotoError::Limit("photo limit of 1 reached".into()))
        );
        assert!(photos.remove(id) && !photos.remove(id));
        assert_eq!(photos.usage(), (0, 0));
    }
}
