//! SigmaOS-audio-tuner — EQ/preamp CLI.
//!
//! Usage:
//!   sigmaos-audio-tuner status|eq|presets
//!   sigmaos-audio-tuner preset <name>          apply builtin/custom preset
//!   sigmaos-audio-tuner save <name> [desc]     save current as preset
//!   sigmaos-audio-tuner preamp <db>            -20..+20
//!   sigmaos-audio-tuner band add <hz> <gain> <q> [label]
//!   sigmaos-audio-tuner band rm <idx>|gain <idx> <db>|clear
//!   sigmaos-audio-tuner loudness [on|off|toggle]
//!   sigmaos-audio-tuner auto-eq <on|off>
//!   sigmaos-audio-tuner device save|load <name>

use audio_tuner::{EqBand, SigmaOSAudioTuner};
use std::process::exit;

fn usage() -> ! {
    eprintln!("usage: sigmaos-audio-tuner <status|eq|presets|preset|save|preamp|band|loudness|auto-eq|device> [args]");
    exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() { usage(); }
    let state = SigmaOSAudioTuner::state_path();
    let mut m = SigmaOSAudioTuner::load_from(&state).unwrap_or_else(|_| SigmaOSAudioTuner::new());
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let arg = |i: usize| -> &str { rest.get(i).map(|s| s.as_str()).unwrap_or_else(|| usage()) };

    let result: Result<(), String> = (|| -> Result<(), String> { match cmd {
        "status" => { println!("{}", m.status_line()); Ok(()) }
        "eq" => { print!("{}", m.render_eq()); Ok(()) }
        "presets" => {
            for p in m.presets() { println!("{:<14} {}", p.name, p.description); }
            Ok(())
        }
        "preset" => { m.apply_preset(arg(0)).map_err(|e| e.to_string())?; println!("preset={}", arg(0)); Ok(()) }
        "save" => {
            let name = arg(0);
            let desc = rest.get(1).cloned().unwrap_or_default();
            m.save_preset(name, &desc).map_err(|e| e.to_string())?;
            println!("saved preset {name}"); Ok(())
        }
        "preamp" => {
            let db: f64 = arg(0).parse().map_err(|_| "bad dB")?;
            m.set_preamp(db).map_err(|e| e.to_string())?;
            println!("preamp={db:+.1}dB"); Ok(())
        }
        "band" => {
            match rest.first().map(|s| s.as_str()) {
                Some("add") => {
                    let hz: f64 = arg(1).parse().map_err(|_| "bad hz")?;
                    let g: f64 = arg(2).parse().map_err(|_| "bad gain")?;
                    let q: f64 = arg(3).parse().map_err(|_| "bad q")?;
                    let label = rest.get(4).cloned();
                    let mut b = EqBand::new(hz, g, q);
                    if let Some(l) = label { b = b.labeled(l); }
                    let idx = m.add_band(b).map_err(|e| e.to_string())?;
                    println!("band {idx} added"); Ok(())
                }
                Some("rm") | Some("remove") => {
                    let idx: usize = arg(1).parse().map_err(|_| "bad idx")?;
                    m.remove_band(idx).map_err(|e| e.to_string())?;
                    println!("band {idx} removed"); Ok(())
                }
                Some("gain") => {
                    let idx: usize = arg(1).parse().map_err(|_| "bad idx")?;
                    let g: f64 = arg(2).parse().map_err(|_| "bad gain")?;
                    m.set_band_gain(idx, g).map_err(|e| e.to_string())?;
                    println!("band {idx} gain={g:+.1}dB"); Ok(())
                }
                Some("clear") => { m.clear_bands(); println!("bands cleared"); Ok(()) }
                _ => usage(),
            }
        }
        "loudness" => {
            match rest.first().map(|s| s.as_str()) {
                Some("on") => { m.set_loudness(true); println!("loudness=on"); }
                Some("off") => { m.set_loudness(false); println!("loudness=off"); }
                _ => { let v = m.toggle_loudness(); println!("loudness={v}"); }
            }
            Ok(())
        }
        "auto-eq" => {
            let on = match rest.first().map(|s| s.as_str()) {
                Some("on")|Some("1")|Some("true") => true,
                _ => false,
            };
            m.set_auto_eq(on); println!("auto_eq={on}"); Ok(())
        }
        "device" => {
            match (rest.first().map(|s| s.as_str()), rest.get(1).map(|s| s.as_str())) {
                (Some("save"), Some(name)) => { m.save_device(name); println!("saved device {name}"); Ok(()) }
                (Some("load"), Some(name)) => {
                    if m.load_device(name) { println!("loaded device {name}"); Ok(()) }
                    else { Err(format!("no saved state for {name}")) }
                }
                _ => usage(),
            }
        }
        _ => usage(),
    } })();

    match result {
        Ok(()) => { if let Err(e) = m.save_to(&state) { eprintln!("sigmaos-audio-tuner: warn: save: {e}"); } }
        Err(e) => { eprintln!("sigmaos-audio-tuner: {e}"); exit(1); }
    }
}
