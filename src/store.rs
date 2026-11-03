//! Session file under `%LOCALAPPDATA%\Clocks\session.json`.
//!
//! Every clock process upserts its [`ClockConfig`] into `open`. Closing one
//! window moves it to `closed` (newest first, capped at 10). **Close all and
//! exit** leaves `open` in place so the next launch can restore the session.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateMutexW, ReleaseMutex, WaitForSingleObject,
};

use crate::config::ClockConfig;

pub const CLOSED_LIMIT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub version: u32,
    #[serde(default)]
    pub open: Vec<ClockConfig>,
    #[serde(default)]
    pub closed: Vec<ClockConfig>,
}

impl SessionState {
    fn normalize(&mut self) {
        self.version = 1;
        if self.closed.len() > CLOSED_LIMIT {
            self.closed.truncate(CLOSED_LIMIT);
        }
    }
}

#[derive(Debug)]
pub struct StoreError;

/// File-backed session store. Cross-process writes take `Local\ClocksSession`.
#[derive(Clone, Debug)]
pub struct FileSessionStore {
    path: PathBuf,
}

impl FileSessionStore {
    pub fn user() -> Self {
        Self {
            path: user_session_path(),
        }
    }

    #[cfg(test)]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[cfg(test)]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn load(&self) -> Result<SessionState, StoreError> {
        with_session_lock(|| self.load_unlocked())
    }

    #[cfg(test)]
    pub fn save(&self, state: &SessionState) -> Result<(), StoreError> {
        with_session_lock(|| self.save_unlocked(state))
    }

    pub fn upsert_open(&self, config: &ClockConfig) -> Result<(), StoreError> {
        if config.id.is_empty() {
            return Err(StoreError);
        }
        with_session_lock(|| {
            let mut state = self.load_unlocked()?;
            if let Some(slot) = state.open.iter_mut().find(|c| c.id == config.id) {
                *slot = config.clone();
            } else {
                state.open.push(config.clone());
            }
            state.closed.retain(|c| c.id != config.id);
            self.save_unlocked(&state)
        })
    }

    /// Remove from `open` and push onto `closed` (newest first).
    pub fn close_to_history(&self, mut config: ClockConfig) -> Result<(), StoreError> {
        config.ensure_id();
        with_session_lock(|| {
            let mut state = self.load_unlocked()?;
            state.open.retain(|c| c.id != config.id);
            state.closed.retain(|c| c.id != config.id);
            state.closed.insert(0, config);
            if state.closed.len() > CLOSED_LIMIT {
                state.closed.truncate(CLOSED_LIMIT);
            }
            self.save_unlocked(&state)
        })
    }

    /// Take a closed clock back into `open` and return it.
    pub fn restore_closed(&self, id: &str) -> Result<Option<ClockConfig>, StoreError> {
        with_session_lock(|| {
            let mut state = self.load_unlocked()?;
            let Some(index) = state.closed.iter().position(|c| c.id == id) else {
                return Ok(None);
            };
            let config = state.closed.remove(index);
            if let Some(slot) = state.open.iter_mut().find(|c| c.id == config.id) {
                *slot = config.clone();
            } else {
                state.open.push(config.clone());
            }
            self.save_unlocked(&state)?;
            Ok(Some(config))
        })
    }

    pub fn closed_list(&self) -> Result<Vec<ClockConfig>, StoreError> {
        Ok(self.load()?.closed)
    }

    fn load_unlocked(&self) -> Result<SessionState, StoreError> {
        if !self.path.exists() {
            return Ok(SessionState {
                version: 1,
                ..SessionState::default()
            });
        }
        let text = fs::read_to_string(&self.path).map_err(|_| StoreError)?;
        let mut state: SessionState = serde_json::from_str(&text).map_err(|_| StoreError)?;
        state.normalize();
        Ok(state)
    }

    fn save_unlocked(&self, state: &SessionState) -> Result<(), StoreError> {
        let mut state = state.clone();
        state.normalize();
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|_| StoreError)?;
        }
        let text = serde_json::to_string_pretty(&state).map_err(|_| StoreError)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|_| StoreError)?;
        fs::rename(&tmp, &self.path).map_err(|_| StoreError)?;
        Ok(())
    }
}

pub fn user_session_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Clocks").join("session.json")
}

fn with_session_lock<T>(body: impl FnOnce() -> Result<T, StoreError>) -> Result<T, StoreError> {
    let mutex =
        unsafe { CreateMutexW(None, false, w!("Local\\ClocksSession")) }.map_err(|_| StoreError)?;
    // Bounded wait avoids a permanent hang if another process died mid-write or
    // the tray thread and UI contended in the wrong order.
    const LOCK_MS: u32 = 3_000;
    let status = unsafe { WaitForSingleObject(mutex, LOCK_MS) };
    if status != WAIT_OBJECT_0 && status != windows::Win32::Foundation::WAIT_ABANDONED {
        let _ = unsafe { CloseHandle(mutex) };
        return Err(StoreError);
    }
    let result = body();
    unsafe {
        let _ = ReleaseMutex(mutex);
        let _ = CloseHandle(mutex);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FaceMode, Language, ThemeMode, WindowRect, ZoneChoice};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample(id: &str) -> ClockConfig {
        ClockConfig {
            id: id.to_string(),
            zone: ZoneChoice::Fixed { seconds: 3600 },
            face: FaceMode::Digital,
            show_seconds: true,
            show_date: true,
            hour24: true,
            always_on_top: false,
            theme: ThemeMode::System,
            language: Language::En,
            background: None,
            font_color: None,
            dial_color: None,
            second_hand_color: None,
            window: WindowRect::default(),
        }
    }

    fn temp_store() -> FileSessionStore {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("clocks-session-test-{nanos}.json"));
        let _ = fs::remove_file(&path);
        FileSessionStore::at(path)
    }

    #[test]
    fn upsert_updates_same_id() {
        let store = temp_store();
        let mut first = sample("a");
        store.upsert_open(&first).unwrap();
        first.window.x = 99.0;
        store.upsert_open(&first).unwrap();
        let state = store.load().unwrap();
        assert_eq!(state.open.len(), 1);
        assert_eq!(state.open[0].window.x, 99.0);
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn close_to_history_keeps_ten_newest() {
        let store = temp_store();
        for i in 0..12 {
            store.close_to_history(sample(&format!("c{i}"))).unwrap();
        }
        let state = store.load().unwrap();
        assert_eq!(state.closed.len(), CLOSED_LIMIT);
        assert_eq!(state.closed[0].id, "c11");
        assert_eq!(state.closed[9].id, "c2");
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn restore_closed_moves_back_to_open() {
        let store = temp_store();
        store.upsert_open(&sample("live")).unwrap();
        store.close_to_history(sample("gone")).unwrap();
        let restored = store.restore_closed("gone").unwrap().unwrap();
        assert_eq!(restored.id, "gone");
        let state = store.load().unwrap();
        assert!(state.closed.iter().all(|c| c.id != "gone"));
        assert!(state.open.iter().any(|c| c.id == "gone"));
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn session_roundtrips_through_json() {
        let store = temp_store();
        let state = SessionState {
            version: 1,
            open: vec![sample("o1")],
            closed: vec![sample("c1")],
        };
        store.save(&state).unwrap();
        assert_eq!(store.load().unwrap(), state);
        let _ = fs::remove_file(store.path());
    }
}
