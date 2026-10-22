//! Persistence seam.
//!
//! Startup does not call this module. A later version can replace
//! [`NullClockStore`] with a file-backed store and spawn one process per
//! saved [`ClockConfig`](crate::config::ClockConfig). The clock window already
//! treats that struct as its only state, so the face, search, and tray do not
//! need to change.

#![allow(dead_code)]

use crate::config::ClockConfig;

#[derive(Debug)]
pub struct StoreError;

pub trait ClockStore {
    fn load(&self) -> Result<Vec<ClockConfig>, StoreError>;
    fn save(&self, clocks: &[ClockConfig]) -> Result<(), StoreError>;
}

/// Store that forgets everything. `load` is empty and `save` is a no-op.
#[derive(Debug, Default)]
pub struct NullClockStore;

impl ClockStore for NullClockStore {
    fn load(&self) -> Result<Vec<ClockConfig>, StoreError> {
        Ok(Vec::new())
    }

    fn save(&self, _clocks: &[ClockConfig]) -> Result<(), StoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClockConfig, FaceMode, Language, ThemeMode, WindowRect, ZoneChoice};

    fn sample() -> ClockConfig {
        ClockConfig {
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

    #[test]
    fn null_store_keeps_nothing() {
        let store = NullClockStore;
        assert!(store.load().unwrap().is_empty());
        store.save(&[sample()]).unwrap();
        assert!(store.load().unwrap().is_empty());
    }
}
