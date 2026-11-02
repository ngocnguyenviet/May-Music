use crate::model::Library;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub struct Store(Connection);

pub fn data_dir() -> PathBuf {
    if let Some(path) = env::var_os("MAY_DATA_DIR") {
        return path.into();
    }
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    base.join("may-music")
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let db = Connection::open(path).context("Không mở được thư viện nhạc")?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS library (id INTEGER PRIMARY KEY CHECK (id = 1), data TEXT NOT NULL);")?;
        Ok(Self(db))
    }
    pub fn load(&self) -> Result<Library> {
        let mut query = self.0.prepare("SELECT data FROM library WHERE id=1")?;
        let mut rows = query.query([])?;
        match rows.next()? {
            Some(row) => serde_json::from_str(&row.get::<_, String>(0)?)
                .context("Dữ liệu playlist bị lỗi; hãy giữ bản sao database trước khi sửa"),
            None => Ok(Library::default()),
        }
    }
    pub fn save(&self, library: &Library) -> Result<()> {
        let data = serde_json::to_string(library)?;
        self.0.execute("INSERT INTO library(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data", [data])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_unicode_playlists_settings_and_last_track() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let store = Store::open(&path).unwrap();
        let mut lib = store.load().unwrap();
        lib.create("Nhạc buổi tối ☁");
        lib.add(crate::model::track("abcdefghijk"));
        lib.autoplay = true;
        lib.volume = 35.0;
        lib.last_track = Some(crate::model::track("abcdefghijk"));
        store.save(&lib).unwrap();
        drop(store);
        let restored = Store::open(&path).unwrap().load().unwrap();
        assert_eq!(restored.selected().unwrap().name, "Nhạc buổi tối ☁");
        assert_eq!(restored.selected().unwrap().tracks.len(), 1);
        assert!(restored.autoplay);
        assert_eq!(restored.volume, 35.0);
        assert_eq!(restored.last_track.unwrap().id, "abcdefghijk");
    }
}
