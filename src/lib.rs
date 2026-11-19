// SPDX-License-Identifier: MIT OR Apache-2.0
//! SigmaOS-audio-tuner — Audio device tuner (EQ, profiles).
//!
//! Provides multi-band parametric equalizer curve calculation, audio profile presets,
//! device gain mapping, and validation.
//!
//! Inspired by Omarchy's audio workstation simplicity, Hyprland's low-latency audio sync,
//! and Catppuccin Mocha's warm acoustic balance.
//!
//! Principles: clamp-bounded dB gains, pure `std`, zero `unsafe`.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

// ── Error types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TunerError {
    GainOutOfRange { band: u32, gain_db: f32 },
    ProfileNotFound(String),
    DeviceNotFound(String),
    InvalidBandFrequency(u32),
    EmptyProfileName,
}

impl fmt::Display for TunerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GainOutOfRange { band, gain_db } => {
                write!(f, "gain {gain_db} dB for band {band} Hz exceeds limits (-12.0..=+12.0 dB)")
            }
            Self::ProfileNotFound(p) => write!(f, "audio profile '{p}' not found"),
            Self::DeviceNotFound(d) => write!(f, "audio device '{d}' not found"),
            Self::InvalidBandFrequency(freq) => write!(f, "unsupported EQ frequency: {freq} Hz"),
            Self::EmptyProfileName => write!(f, "profile name cannot be empty"),
        }
    }
}

impl std::error::Error for TunerError {}

// ── Core Domain Types ────────────────────────────────────────────────────────

pub const STANDARD_BANDS: [u32; 10] = [31, 63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

#[derive(Debug, Clone, PartialEq)]
pub struct EqBand {
    pub frequency_hz: u32,
    pub gain_db: f32,
}

impl EqBand {
    pub fn new(freq: u32, gain: f32) -> Result<Self, TunerError> {
        if !STANDARD_BANDS.contains(&freq) {
            return Err(TunerError::InvalidBandFrequency(freq));
        }
        if !(-12.0..=12.0).contains(&gain) {
            return Err(TunerError::GainOutOfRange {
                band: freq,
                gain_db: gain,
            });
        }
        Ok(Self {
            frequency_hz: freq,
            gain_db: gain,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioProfile {
    pub name: String,
    pub bands: HashMap<u32, f32>,
    pub preamp_db: f32,
}

impl AudioProfile {
    pub fn new(name: impl Into<String>) -> Result<Self, TunerError> {
        let name = name.into().trim().to_string();
        if name.is_empty() {
            return Err(TunerError::EmptyProfileName);
        }
        let mut bands = HashMap::new();
        for &b in &STANDARD_BANDS {
            bands.insert(b, 0.0);
        }
        Ok(Self {
            name,
            bands,
            preamp_db: 0.0,
        })
    }

    pub fn set_band(&mut self, freq: u32, gain: f32) -> Result<(), TunerError> {
        let band = EqBand::new(freq, gain)?;
        self.bands.insert(band.frequency_hz, band.gain_db);
        Ok(())
    }

    pub fn flat() -> Self {
        Self::new("Flat").unwrap()
    }

    pub fn bass_boost() -> Self {
        let mut p = Self::new("Bass Boost").unwrap();
        p.set_band(31, 6.0).unwrap();
        p.set_band(63, 5.0).unwrap();
        p.set_band(125, 3.5).unwrap();
        p
    }

    pub fn vocal_clarity() -> Self {
        let mut p = Self::new("Vocal Clarity").unwrap();
        p.set_band(1000, 2.5).unwrap();
        p.set_band(2000, 3.5).unwrap();
        p.set_band(4000, 2.0).unwrap();
        p
    }

    pub fn catppuccin_warm() -> Self {
        let mut p = Self::new("Catppuccin Warm").unwrap();
        p.set_band(125, 2.0).unwrap();
        p.set_band(250, 1.5).unwrap();
        p.set_band(8000, -1.0).unwrap();
        p.set_band(16000, -2.0).unwrap();
        p
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub active_profile: String,
    pub master_volume: f32, // 0.0..=1.0
}

impl AudioDevice {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            active_profile: "Flat".into(),
            master_volume: 0.8,
        }
    }
}

#[derive(Debug, Default)]
pub struct TunerManager {
    profiles: HashMap<String, AudioProfile>,
    devices: HashMap<String, AudioDevice>,
}

impl TunerManager {
    pub fn new() -> Self {
        let mut m = Self::default();
        m.add_profile(AudioProfile::flat()).unwrap();
        m.add_profile(AudioProfile::bass_boost()).unwrap();
        m.add_profile(AudioProfile::vocal_clarity()).unwrap();
        m.add_profile(AudioProfile::catppuccin_warm()).unwrap();
        m
    }

    pub fn add_profile(&mut self, profile: AudioProfile) -> Result<(), TunerError> {
        self.profiles.insert(profile.name.clone(), profile);
        Ok(())
    }

    pub fn get_profile(&self, name: &str) -> Result<&AudioProfile, TunerError> {
        self.profiles.get(name).ok_or_else(|| TunerError::ProfileNotFound(name.into()))
    }

    pub fn register_device(&mut self, dev: AudioDevice) {
        self.devices.insert(dev.id.clone(), dev);
    }

    pub fn apply_profile_to_device(&mut self, device_id: &str, profile_name: &str) -> Result<(), TunerError> {
        if !self.profiles.contains_key(profile_name) {
            return Err(TunerError::ProfileNotFound(profile_name.into()));
        }
        let dev = self.devices.get_mut(device_id).ok_or_else(|| TunerError::DeviceNotFound(device_id.into()))?;
        dev.active_profile = profile_name.to_string();
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_band_valid_and_out_of_range() {
        // Arrange & Act
        let ok = EqBand::new(1000, 3.5);
        let too_high = EqBand::new(1000, 15.0);
        let invalid_freq = EqBand::new(999, 0.0);

        // Assert
        assert!(ok.is_ok());
        assert_eq!(too_high, Err(TunerError::GainOutOfRange { band: 1000, gain_db: 15.0 }));
        assert_eq!(invalid_freq, Err(TunerError::InvalidBandFrequency(999)));
    }

    #[test]
    fn audio_profile_empty_name_fails() {
        // Arrange & Act
        let err = AudioProfile::new("  ");
        // Assert
        assert_eq!(err, Err(TunerError::EmptyProfileName));
    }

    #[test]
    fn presets_initialised_with_expected_bands() {
        // Arrange & Act
        let bass = AudioProfile::bass_boost();
        let warm = AudioProfile::catppuccin_warm();

        // Assert
        assert_eq!(bass.bands.get(&31), Some(&6.0));
        assert_eq!(warm.bands.get(&16000), Some(&-2.0));
    }

    #[test]
    fn manager_default_presets_available() {
        // Arrange
        let mgr = TunerManager::new();
        // Act & Assert
        assert!(mgr.get_profile("Flat").is_ok());
        assert!(mgr.get_profile("Bass Boost").is_ok());
        assert!(mgr.get_profile("Catppuccin Warm").is_ok());
    }

    #[test]
    fn apply_profile_to_device_success_and_failure() {
        // Arrange
        let mut mgr = TunerManager::new();
        let dev = AudioDevice::new("sink-1", "Headphones");
        mgr.register_device(dev);

        // Act
        let ok = mgr.apply_profile_to_device("sink-1", "Bass Boost");
        let bad_dev = mgr.apply_profile_to_device("sink-99", "Bass Boost");
        let bad_prof = mgr.apply_profile_to_device("sink-1", "UnknownPreset");

        // Assert
        assert!(ok.is_ok());
        assert_eq!(bad_dev, Err(TunerError::DeviceNotFound("sink-99".into())));
        assert_eq!(bad_prof, Err(TunerError::ProfileNotFound("UnknownPreset".into())));
    }

    #[test]
    fn error_display_formatting() {
        // Arrange
        let err = TunerError::GainOutOfRange { band: 500, gain_db: 20.0 };
        // Act & Assert
        assert_eq!(format!("{err}"), "gain 20 dB for band 500 Hz exceeds limits (-12.0..=+12.0 dB)");
    }
}
