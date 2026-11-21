// SPDX-License-Identifier: Apache-2.0
//! SigmaOS-audio-tuner — EQ/preamp/loudness audio tuner.
//!
//! Models a parametric EQ (frequency bands with gain/Q), preamp, and
//! loudness/normalization targets — the policy layer `pactl`/`wpctl`
//! doesn't cover. Bands are applied to a named sink/source; presets
//! (flat, bass-boost, vocal, night) are first-class. Persistence via
//! JSON. A real backend writes to PipeWire/EasyEffects.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EqBand {
    pub freq_hz: f64,      // center frequency
    pub gain_db: f64,      // -20..+20
    pub q: f64,            // quality 0.1..10
    pub label: String,
}
impl EqBand {
    pub fn new(freq_hz: f64, gain_db: f64, q: f64) -> Self {
        Self { freq_hz, gain_db, q, label: format!("{:.0}Hz", freq_hz) }
    }
    pub fn labeled(mut self, l: impl Into<String>) -> Self { self.label = l.into(); self }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TunerPreset {
    pub name: String,
    pub description: String,
    pub preamp_db: f64,
    pub bands: Vec<EqBand>,
    pub loudness: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TunerError {
    BadFreq(f64),
    BadGain(f64),
    BadQ(f64),
    BadPreamp(f64),
    NoSuchPreset(String),
    DuplicatePreset(String),
}
impl fmt::Display for TunerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadFreq(v)=>write!(f,"freq {v} out of range (20..20000)"),
            Self::BadGain(v)=>write!(f,"gain {v}dB out of range (-20..+20)"),
            Self::BadQ(v)=>write!(f,"Q {v} out of range (0.1..10)"),
            Self::BadPreamp(v)=>write!(f,"preamp {v}dB out of range (-20..+20)"),
            Self::NoSuchPreset(n)=>write!(f,"no preset {n}"),
            Self::DuplicatePreset(n)=>write!(f,"duplicate preset {n}"),
        }
    }
}
impl std::error::Error for TunerError {}

fn validate_band(b: &EqBand) -> Result<(), TunerError> {
    if !(20.0..=20000.0).contains(&b.freq_hz) { return Err(TunerError::BadFreq(b.freq_hz)); }
    if !(-20.0..=20.0).contains(&b.gain_db) { return Err(TunerError::BadGain(b.gain_db)); }
    if !(0.1..=10.0).contains(&b.q) { return Err(TunerError::BadQ(b.q)); }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TunerState {
    pub preamp_db: f64,
    pub bands: Vec<EqBand>,
    pub loudness: bool,
    pub active_preset: Option<String>,
    pub auto_eq: bool,          // auto-adjust per device profile
}
impl Default for TunerState {
    fn default() -> Self {
        Self { preamp_db: 0.0, bands: Vec::new(), loudness: false,
               active_preset: None, auto_eq: false }
    }
}

pub struct SigmaOSAudioTuner {
    state: TunerState,
    presets: BTreeMap<String, TunerPreset>,
    /// device name -> saved per-device state
    device_states: BTreeMap<String, TunerState>,
}

impl SigmaOSAudioTuner {
    pub fn new() -> Self {
        let mut m = Self { state: TunerState::default(), presets: BTreeMap::new(), device_states: BTreeMap::new() };
        m.seed_presets();
        m
    }

    fn seed_presets(&mut self) {
        let flat = TunerPreset { name: "flat".into(), description: "no EQ".into(),
            preamp_db: 0.0, bands: vec![], loudness: false };
        let bass = TunerPreset { name: "bass-boost".into(), description: "+6dB low shelf".into(),
            preamp_db: -2.0, loudness: true,
            bands: vec![
                EqBand::new(80.0, 6.0, 0.8).labeled("low-shelf"),
                EqBand::new(200.0, 3.0, 1.0).labeled("low-mid"),
            ] };
        let vocal = TunerPreset { name: "vocal".into(), description: "presence boost".into(),
            preamp_db: -1.0, loudness: false,
            bands: vec![
                EqBand::new(250.0, -2.0, 1.0).labeled("reduce mud"),
                EqBand::new(3000.0, 4.0, 2.0).labeled("presence"),
                EqBand::new(8000.0, 2.0, 1.5).labeled("air"),
            ] };
        let night = TunerPreset { name: "night".into(), description: "compressed loudness for quiet".into(),
            preamp_db: -8.0, loudness: true,
            bands: vec![EqBand::new(1000.0, 2.0, 0.7).labeled("presence-comp")] };
        for p in [flat, bass, vocal, night] { self.presets.insert(p.name.clone(), p); }
    }

    pub fn state(&self) -> &TunerState { &self.state }
    pub fn presets(&self) -> impl Iterator<Item = &TunerPreset> { self.presets.values() }
    pub fn preset(&self, name: &str) -> Option<&TunerPreset> { self.presets.get(name) }

    // ---- band management ----
    pub fn set_preamp(&mut self, db: f64) -> Result<f64, TunerError> {
        if !(-20.0..=20.0).contains(&db) { return Err(TunerError::BadPreamp(db)); }
        self.state.preamp_db = db;
        self.state.active_preset = None; // manual change clears preset
        Ok(db)
    }

    pub fn add_band(&mut self, band: EqBand) -> Result<usize, TunerError> {
        validate_band(&band)?;
        self.state.bands.push(band);
        self.state.active_preset = None;
        Ok(self.state.bands.len() - 1)
    }

    pub fn remove_band(&mut self, idx: usize) -> Result<EqBand, TunerError> {
        if idx >= self.state.bands.len() { return Err(TunerError::NoSuchPreset(format!("band {idx}"))); }
        self.state.active_preset = None;
        Ok(self.state.bands.remove(idx))
    }

    pub fn set_band_gain(&mut self, idx: usize, gain_db: f64) -> Result<(), TunerError> {
        let b = self.state.bands.get_mut(idx).ok_or(TunerError::NoSuchPreset(format!("band {idx}")))?;
        if !(-20.0..=20.0).contains(&gain_db) { return Err(TunerError::BadGain(gain_db)); }
        b.gain_db = gain_db; self.state.active_preset = None; Ok(())
    }

    pub fn clear_bands(&mut self) {
        self.state.bands.clear();
        self.state.active_preset = None;
    }

    pub fn set_loudness(&mut self, on: bool) { self.state.loudness = on; self.state.active_preset = None; }
    pub fn toggle_loudness(&mut self) -> bool { self.state.loudness = !self.state.loudness; self.state.loudness }
    pub fn set_auto_eq(&mut self, on: bool) { self.state.auto_eq = on; }

    // ---- presets ----
    pub fn apply_preset(&mut self, name: &str) -> Result<(), TunerError> {
        let p = self.presets.get(name).cloned().ok_or_else(|| TunerError::NoSuchPreset(name.into()))?;
        self.state.preamp_db = p.preamp_db;
        self.state.bands = p.bands;
        self.state.loudness = p.loudness;
        self.state.active_preset = Some(name.into());
        Ok(())
    }

    pub fn save_preset(&mut self, name: &str, description: &str) -> Result<(), TunerError> {
        if self.presets.contains_key(name) { return Err(TunerError::DuplicatePreset(name.into())); }
        let p = TunerPreset { name: name.into(), description: description.into(),
            preamp_db: self.state.preamp_db, bands: self.state.bands.clone(),
            loudness: self.state.loudness };
        self.presets.insert(name.into(), p);
        Ok(())
    }

    // ---- per-device ----
    pub fn save_device(&mut self, device: &str) {
        self.device_states.insert(device.to_string(), self.state.clone());
    }
    pub fn load_device(&mut self, device: &str) -> bool {
        if let Some(s) = self.device_states.get(device) {
            self.state = s.clone(); true
        } else { false }
    }
    pub fn device_count(&self) -> usize { self.device_states.len() }

    /// Approximate perceived loudness change in dB (preamp + band sum at 1kHz ref).
    pub fn loudness_delta_db(&self) -> f64 {
        let band_sum: f64 = self.state.bands.iter().map(|b| b.gain_db).sum::<f64>() / self.state.bands.len().max(1) as f64;
        self.state.preamp_db + if self.state.loudness { 3.0 } else { 0.0 } + band_sum * 0.3
    }

    pub fn render_eq(&self) -> String {
        let mut out = format!("preamp {:.1}dB loudness={} auto_eq={}\n", self.state.preamp_db,
            self.state.loudness, self.state.auto_eq);
        for b in &self.state.bands {
            out.push_str(&format!("  {:<12} {:>7.0}Hz gain {:+.1}dB Q {:.1}\n", b.label, b.freq_hz, b.gain_db, b.q));
        }
        out
    }

    /// Persist state + custom presets + per-device states to JSON.
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Disk {
            state: TunerState,
            custom_presets: Vec<TunerPreset>,
            device_states: BTreeMap<String, TunerState>,
        }
        // presets minus builtins (builtin names are reserved)
        let builtin = ["flat","bass-boost","vocal","night"];
        let custom: Vec<TunerPreset> = self.presets.values()
            .filter(|p| !builtin.contains(&p.name.as_str())).cloned().collect();
        let d = Disk { state: self.state.clone(), custom_presets: custom,
                       device_states: self.device_states.clone() };
        let j = serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?;
        if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        std::fs::write(path, j).map_err(|e| e.to_string())
    }

    /// Load persisted state; seeds builtins fresh, overlays saved state.
    pub fn load_from(path: &std::path::Path) -> Result<Self, String> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Disk {
            state: TunerState,
            custom_presets: Vec<TunerPreset>,
            device_states: BTreeMap<String, TunerState>,
        }
        let txt = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let d: Disk = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let mut m = Self::new();
        for p in d.custom_presets { m.presets.insert(p.name.clone(), p); }
        m.state = d.state;
        m.device_states = d.device_states;
        Ok(m)
    }

    pub fn state_path() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("SIGMAOS_TUNER_FILE") { return std::path::PathBuf::from(p); }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        std::path::PathBuf::from(home).join(".local/share/sigmaos/audio-tuner.json")
    }

    pub fn status_line(&self) -> String {
        format!("tuner: preset={} bands={} preamp={:+.1}dB loudness={} Δ{:+.1}dB",
            self.state.active_preset.as_deref().unwrap_or("custom"),
            self.state.bands.len(), self.state.preamp_db, self.state.loudness,
            self.loudness_delta_db())
    }
}

impl Default for SigmaOSAudioTuner {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_presets() {
        let m = SigmaOSAudioTuner::new();
        assert!(m.preset("flat").is_some());
        assert!(m.preset("bass-boost").is_some());
        assert!(m.preset("vocal").is_some());
        assert!(m.preset("night").is_some());
    }

    #[test]
    fn band_validation() {
        let mut m = SigmaOSAudioTuner::new();
        assert!(m.add_band(EqBand::new(10.0, 0.0, 1.0)).is_err()); // freq too low
        assert!(m.add_band(EqBand::new(1000.0, 25.0, 1.0)).is_err()); // gain too high
        assert!(m.add_band(EqBand::new(1000.0, 0.0, 20.0)).is_err()); // Q too high
        assert!(m.add_band(EqBand::new(1000.0, 3.0, 1.0)).is_ok());
        assert_eq!(m.state().bands.len(), 1);
    }

    #[test]
    fn preamp_and_loudness() {
        let mut m = SigmaOSAudioTuner::new();
        assert_eq!(m.set_preamp(-6.0).unwrap(), -6.0);
        assert!(m.set_preamp(30.0).is_err());
        m.set_loudness(true);
        assert!(m.state().loudness);
        assert!(!m.toggle_loudness()); // toggles to false, returns new value
        assert!(!m.state().loudness);
        assert!(m.toggle_loudness()); // back to true
    }

    #[test]
    fn presets_apply_and_clear() {
        let mut m = SigmaOSAudioTuner::new();
        m.apply_preset("bass-boost").unwrap();
        assert_eq!(m.state().active_preset.as_deref(), Some("bass-boost"));
        assert_eq!(m.state().bands.len(), 2);
        assert!(m.state().loudness);
        // manual change clears preset
        m.set_preamp(0.0).unwrap();
        assert_eq!(m.state().active_preset, None);
        assert!(m.apply_preset("nope").is_err());
    }

    #[test]
    fn save_custom_preset() {
        let mut m = SigmaOSAudioTuner::new();
        m.set_preamp(-3.0).unwrap();
        m.add_band(EqBand::new(500.0, 2.0, 1.0)).unwrap();
        m.save_preset("my-eq", "custom").unwrap();
        assert!(m.preset("my-eq").is_some());
        assert!(m.save_preset("my-eq", "dup").is_err());
        // apply it fresh
        m.clear_bands(); m.set_preamp(0.0).unwrap();
        m.apply_preset("my-eq").unwrap();
        assert_eq!(m.state().bands.len(), 1);
    }

    #[test]
    fn per_device_states() {
        let mut m = SigmaOSAudioTuner::new();
        m.apply_preset("vocal").unwrap();
        m.save_device("headphones");
        m.apply_preset("night").unwrap();
        assert!(m.load_device("headphones"));
        assert_eq!(m.state().active_preset.as_deref(), Some("vocal"));
        assert!(!m.load_device("speakers"));
        assert_eq!(m.device_count(), 1);
    }

    #[test]
    fn render_and_status() {
        let mut m = SigmaOSAudioTuner::new();
        m.apply_preset("vocal").unwrap();
        let r = m.render_eq();
        assert!(r.contains("preamp") && r.contains("3000Hz") && r.contains("presence"));
        assert!(m.status_line().contains("vocal"));
    }
}
