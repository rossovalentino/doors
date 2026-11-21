//! P15 SOUND-A: an ordinary Windows console program (`soundprobe.exe`,
//! built with the pinned MSVC toolchain and SDK import libraries, no C
//! runtime) runs from Terminal and plays through winmm.dll: waveOut with
//! function and event callbacks, reset and position, PlaySound from memory
//! and by alias, MessageBeep, error texts, multimedia timers, two devices
//! mixed and an underrun. QEMU's wav backend captures what the guest
//! played: the waveOut tone (660 Hz, two seconds), the PlaySound tone
//! (880 Hz, one second), the mixed pair (440 Hz and 1000 Hz in the same
//! instants) and both underrun bursts (550 Hz) must all be in it.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const SOUNDPROBE: Probe = Probe {
    name: "soundprobe",
    directory: "tests/fixtures/windows-sound",
    libraries: &["kernel32.lib", "User32.Lib", "WinMM.Lib"],
    modules: &["kernel32.dll", "user32.dll", "winmm.dll"],
};
const RESOLUTION: &str = "1280x720";
/// Steps soundprobe reports, in order.
const STEPS: [&str; 15] = [
    "devices",
    "caps",
    "format-query",
    "open-callback",
    "play",
    "position",
    "close",
    "reset",
    "event-callback",
    "playsound-memory",
    "system-sounds",
    "errors",
    "timers",
    "mixed",
    "underrun",
];
/// The frequency of every loud quarter second of a capture, stepping by an
/// eighth of a second, by upward zero crossings of its first channel.
fn quarters(samples: &[i32], rate: u32) -> Vec<u32> {
    let window = rate as usize / 4;
    let mut found = Vec::new();
    let mut start = 0;
    while start + window <= samples.len() {
        let part = &samples[start..start + window];
        // Loud throughout: every sixteenth of a second reaches 1000.
        if part
            .chunks(window / 4)
            .all(|c| c.iter().map(|v| v.abs()).max().unwrap_or(0) > 1000)
        {
            found.push(part.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count() as u32 * 4);
        }
        start += window / 2;
    }
    found
}
/// The amplitude of the `hz` component of `part` (Goertzel).
fn amplitude(part: &[i32], rate: u32, hz: u32) -> f64 {
    let coefficient = 2.0 * (std::f64::consts::TAU * f64::from(hz) / f64::from(rate)).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in part {
        let s = f64::from(x) + coefficient * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let power = s1 * s1 + s2 * s2 - coefficient * s1 * s2;
    2.0 * power.max(0.0).sqrt() / part.len() as f64
}
/// Windows of `seconds`, stepping by half that, in which every one of
/// `tones` has an amplitude of at least 1000.
fn windows_with(samples: &[i32], rate: u32, seconds: f64, tones: &[u32]) -> usize {
    let window = (f64::from(rate) * seconds) as usize;
    (0..samples.len().saturating_sub(window))
        .step_by(window / 2)
        .filter(|&start| {
            tones
                .iter()
                .all(|&hz| amplitude(&samples[start..start + window], rate, hz) >= 1000.0)
        })
        .count()
}
/// Quarter seconds, stepping by an eighth, in which 1480 Hz is loud and 370 Hz
/// is there at -6 dB below it (an amplitude ratio of 0.35..0.7; -6.02 dB is
/// 0.5), as DSOUND-A's mixed step plays them.
fn mixed_at_half(samples: &[i32], rate: u32) -> usize {
    let window = rate as usize / 4;
    (0..samples.len().saturating_sub(window))
        .step_by(window / 2)
        .filter(|&start| {
            let part = &samples[start..start + window];
            let (low, high) = (amplitude(part, rate, 370), amplitude(part, rate, 1480));
            high >= 1000.0 && (0.35..=0.7).contains(&(low / high))
        })
        .count()
}
/// Quarter seconds within 3% of `hz`.
fn near(found: &[u32], hz: u32) -> usize {
    found
        .iter()
        .filter(|f| f.abs_diff(hz) <= hz * 3 / 100)
        .count()
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/sound-a", run.id))?;
    let fixture = build(run, &target, &SOUNDPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-sound",
        &["windows/winmm.dll", "windows/user32.dll"],
    )?;
    let disk = super::win32::fresh_disk(run, "sound-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        let device =
            super::p15_desktop::line_after(run, &record, 0, "DOOR_P15 TRAY AUDIO present=")?;
        if !device.contains("present=1") {
            return Err(format!("no sound device in the guest: {device}"));
        }
        thread::sleep(Duration::from_secs(2));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(1));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "soundprobe.exe",
            "SOUND-A",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "sound-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "sound-a-terminal")?;
        Ok(json!({"probe":probe,"device":device,"screenshot":shot}))
    })();
    let mut details = super::win32::quit_on_error(run, &record, result, "sound-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    // What the guest played, as QEMU's wav backend wrote it.
    let serial_path = run
        .root
        .join(record["serial"].as_str().ok_or("session serial")?);
    let wav = fs::read(serial_path.with_file_name("sound.wav"))
        .map_err(|e| format!("captured sound: {e}"))?;
    let (samples, rate) = super::p15_desktop::pcm16(&wav)?;
    let found = quarters(&samples, rate);
    let (wave_out, play_sound) = (near(&found, 660), near(&found, 880));
    // Two seconds of 660 Hz hold at least six whole quarter-second windows
    // at eighth-second steps, one second of 880 Hz at least two; a second
    // of the mixed pair at least two quarter seconds holding both tones,
    // and each underrun burst at least one whole eighth of a second.
    let mixed = windows_with(&samples, rate, 0.25, &[440, 1000]);
    let bursts = windows_with(&samples, rate, 0.125, &[550]);
    if wave_out < 6 || play_sound < 2 || mixed < 2 || bursts < 2 {
        return Err(format!(
            "captured sound holds {wave_out} quarter seconds of 660 Hz (waveOut), {play_sound} of 880 Hz (PlaySound), {mixed} of 440 Hz with 1000 Hz (mixed) and {bursts} eighths of 550 Hz (underrun); frequencies {found:?}"
        ));
    }
    details["captured"] = json!({"bytes":wav.len(),"quarter_seconds_hz":found,"waveout_660":wave_out,"playsound_880":play_sound,"mixed_440_1000":mixed,"underrun_550":bursts});
    run.check("p15-sound-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "scope":"An MSVC-built Windows console program using winmm.dll (waveOut, PlaySound, MessageBeep through user32, multimedia timers) in the guest, its output captured by QEMU's wav backend"}));
    Ok(())
}

/// Steps `soundprobe.exe capture` reports, in order.
const MIC_STEPS: [&str; 6] = [
    "in-devices",
    "in-open",
    "record",
    "in-position",
    "stop-reset",
    "hold",
];
/// Each `TAG n name ok ...` line of `output` for `steps`, in order.
fn probe_steps(output: &str, tag: &str, steps: &[&str]) -> Result<Vec<Value>, String> {
    let lines: Vec<&str> = output
        .lines()
        .filter(|l| l.starts_with(&format!("{tag} ")))
        .collect();
    let mut found = Vec::new();
    for (index, name) in steps.iter().enumerate() {
        let prefix = format!("{tag} {} {name} ", index + 1);
        let line = lines
            .iter()
            .find(|l| l.starts_with(&prefix))
            .ok_or(format!(
                "{tag} step {} {name} missing from the console",
                index + 1
            ))?;
        found.push(json!({"step":index + 1,"name":name,"line":line.trim_end()}));
    }
    if !output.contains(&format!("{tag} PASSED")) {
        return Err(format!("{tag} failed:\n{}", lines.join("\n")));
    }
    Ok(found)
}

/// P15 MIC-A (SOUND-A capture): QEMU's silent input stands in for a
/// microphone. `soundprobe.exe capture` records through winmm's waveIn;
/// while it holds a recording the taskbar shows the microphone glyph and
/// Settings > Privacy & security > Microphone lists it as in use. Turning
/// off "Let desktop apps access your microphone" there makes
/// `soundprobe.exe denied` fail to open the device (MMSYSERR_ERROR).
pub(super) fn verify_capture(run: &mut Run, image: &str) -> Result<(), String> {
    use door_service_protocol::{audio::capture, preferences as prefs};
    use door_workspace::settings_catalog::{Action, List, Toggle, points};
    use door_workspace::settings_layout::Page;
    let target = run.generated_directory(&format!("out/builds/{}/mic-a", run.id))?;
    let fixture = build(run, &target, &SOUNDPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-mic",
        &["windows/winmm.dll", "windows/user32.dll"],
    )?;
    let disk = super::win32::fresh_disk(run, "mic-a")?;
    let silent = crate::emulator::silent_input();
    let launched = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION);
    drop(silent);
    let mut record = launched?;
    let client = super::administration::maximized_client((1280, 720));
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        let mut p = (640, 360);
        let device =
            super::p15_desktop::line_after(run, &record, 0, "DOOR_P15 TRAY AUDIO present=")?;
        if !device.contains("present=1") {
            return Err(format!("no sound device in the guest: {device}"));
        }
        thread::sleep(Duration::from_secs(2));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(1));
        // MIC-A: record; while it holds, the taskbar and Settings show it.
        let before = session::transcript(&run.root, &record)?.len();
        let offset = serial(&run.root, &record)?.len();
        let line = "soundprobe.exe capture";
        type_line(&mut c, line)?;
        let pid = super::win32::wait_launch(
            run,
            &record,
            offset,
            fixture["sha256"].as_str().ok_or("probe hash")?,
            line,
        )?;
        super::p15_connect::console_after(run, &record, before, "MIC-A holding", 60)?;
        thread::sleep(Duration::from_millis(1500));
        let started =
            super::p15_desktop::line_after(run, &record, offset, "DOOR_P15 AUDIO CAPTURE started")?;
        let frame = scene::capture(run, &mut c, "mic-a-taskbar", &[])?;
        let taskbar = super::win32::keep(run, &frame, "mic-a-taskbar")?;
        // The microphone glyph names the recording app after a dwell, and a
        // click opens Privacy & security > Microphone.
        let mic = door_window::taskbar::Layout::new(1280, 720).microphone();
        let (mx, my) = (
            mic.x() + mic.width() as i32 / 2,
            mic.y() + mic.height() as i32 / 2,
        );
        move_to(&mut c, &mut p, mx, my)?;
        thread::sleep(Duration::from_millis(1500));
        let frame = scene::capture(run, &mut c, "mic-a-tooltip", &[])?;
        let tooltip = super::win32::keep(run, &frame, "mic-a-tooltip")?;
        let before_page = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, mx, my)?;
        super::p15_desktop::line_after(
            run,
            &record,
            before_page,
            &format!("SETTINGS PAGE id={}", Page::Microphone as u32),
        )?;
        key(&mut c, &["meta_l", "up"])?;
        thread::sleep(Duration::from_millis(1500));
        let frame = scene::capture(run, &mut c, "mic-a-settings-in-use", &[])?;
        let in_use = super::win32::keep(run, &frame, "mic-a-settings-in-use")?;
        let output = super::p15_connect::console_after(run, &record, before, "MIC-A PASSED", 60)
            .or_else(|e| {
                super::p15_connect::console_after(run, &record, before, "MIC-A FAILED", 1)
                    .map(|o| format!("{o}\n({e})"))
            })?;
        let steps = probe_steps(&output, "MIC-A", &MIC_STEPS)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let exit = loop {
            if let Some(exit) = super::win32::processes(&serial(&run.root, &record)?, offset)
                .remove(&pid)
                .and_then(|p| p.exit)
            {
                break exit;
            }
            if Instant::now() >= deadline {
                return Err(format!("{line}: pid {pid} did not exit"));
            }
            thread::sleep(Duration::from_millis(100));
        };
        if exit != 0 {
            return Err(format!("MIC-A exited {exit}"));
        }
        thread::sleep(Duration::from_millis(1500));
        let frame = scene::capture_settled(run, &mut c, "mic-a-settings-recent", &[])?;
        let ended = super::win32::keep(run, &frame, "mic-a-settings-recent")?;
        // Turn off "Let desktop apps access your microphone".
        let toggle = Toggle::PrefOff {
            id: prefs::MICROPHONE,
            bit: capture::DESKTOP_OFF,
        };
        let (x, y) = points::item(
            Page::Microphone,
            client.0,
            client.1,
            [0; List::COUNT],
            |i| i.action == Action::Toggle(toggle),
        )
        .ok_or("Microphone > Let desktop apps access your microphone")?;
        let before = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, x, y)?;
        let policy =
            super::p15_desktop::line_after(run, &record, before, "DOOR_P15 AUDIO CAPTURE policy=")?;
        if !policy.contains(&format!("policy={}", capture::DESKTOP_OFF)) {
            return Err(format!("the desktop apps switch sent {policy}"));
        }
        thread::sleep(Duration::from_millis(800));
        let frame = scene::capture_settled(run, &mut c, "mic-a-settings-desktop-off", &[])?;
        let off = super::win32::keep(run, &frame, "mic-a-settings-desktop-off")?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(1200));
        // MIC-D: the device is there, opening it is refused.
        let denied = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "soundprobe.exe denied",
            "MIC-D",
            &["denied"],
        )?;
        Ok(
            json!({"capture":{"command":line,"pid":pid,"exit":exit,"steps":steps,"service":started},
            "denied":denied,"policy":policy,
            "screenshots":{"taskbar":taskbar,"tooltip":tooltip,"settings_in_use":in_use,"settings_recent":ended,"settings_desktop_off":off}}),
        )
    })();
    let details = super::win32::quit_on_error(run, &record, result, "mic-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p15-mic-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "scope":"An MSVC-built Windows console program recording through winmm.dll's waveIn from QEMU's silent input, the taskbar microphone glyph and the Microphone privacy page while it records, and the desktop-apps switch refusing the next open"}));
    Ok(())
}

const DSPROBE: Probe = Probe {
    name: "dsprobe",
    directory: "tests/fixtures/windows-sound",
    libraries: &["kernel32.lib", "User32.Lib", "dsound.lib"],
    modules: &["dsound.dll", "kernel32.dll", "user32.dll"],
};
/// Steps `dsprobe.exe capture` reports, in order.
const DSCAP_STEPS: [&str; 6] = [
    "cap-enumerate",
    "cap-create",
    "cap-buffer",
    "capture",
    "cap-looping",
    "cap-release",
];
/// Steps dsprobe reports, in order.
const DSOUND_STEPS: [&str; 12] = [
    "enumerate",
    "device-id",
    "create",
    "caps",
    "primary",
    "bad-buffers",
    "one-shot",
    "streaming",
    "mixed",
    "frequency",
    "stop-position",
    "release",
];
/// P15 DSOUND-A: `dsprobe.exe` (MSVC, SDK import libraries, no C runtime)
/// plays through dsound.dll from Terminal. QEMU's wav backend captures
/// the one-shot buffer (740 Hz, 1.5 s), the streamed looping buffer
/// (990 Hz, two seconds), the two buffers mixed at once (370 Hz and
/// 1480 Hz, one second) and the 600 Hz tone played at twice its rate
/// (1200 Hz, one second). A second boot, with QEMU's silent input (the wav
/// backend has none), runs `dsprobe.exe capture` (DSCAP-A): recording
/// through DirectSoundCapture.
pub(super) fn verify_dsound(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/dsound-a", run.id))?;
    let fixture = build(run, &target, &DSPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-dsound",
        &["windows/dsound.dll", "windows/user32.dll"],
    )?;
    let session = |run: &mut Run, silent: bool, line: &str, tag: &str, steps: &[&str]| {
        let disk = super::win32::fresh_disk(run, &format!("dsound-a-{}", tag.to_lowercase()))?;
        let guard = silent.then(crate::emulator::silent_input);
        let launched = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION);
        drop(guard);
        let mut record = launched?;
        let shot_name = format!("{}-terminal", tag.to_lowercase());
        let result: Result<Value, String> = (|| {
            let mut c = connection(&record)?;
            let device =
                super::p15_desktop::line_after(run, &record, 0, "DOOR_P15 TRAY AUDIO present=")?;
            if !device.contains("present=1") {
                return Err(format!("no sound device in the guest: {device}"));
            }
            thread::sleep(Duration::from_secs(2));
            super::win32::open_terminal(run, &record, &mut c)?;
            thread::sleep(Duration::from_secs(1));
            let probe = run_probe(run, &record, &mut c, &fixture, line, tag, steps)?;
            let frame = scene::capture_settled(run, &mut c, &shot_name, &[])?;
            let shot = super::win32::keep(run, &frame, &shot_name)?;
            Ok(json!({"probe":probe,"device":device,"screenshot":shot}))
        })();
        let details =
            super::win32::quit_on_error(run, &record, result, &format!("{shot_name}-failure"))?;
        let mut c = connection(&record)?;
        super::win32::finish(run, &mut record, &mut c)?;
        Ok::<_, String>((details, record))
    };
    let (mut details, record) = session(run, false, "dsprobe.exe", "DSOUND-A", &DSOUND_STEPS)?;
    let serial_path = run
        .root
        .join(record["serial"].as_str().ok_or("session serial")?);
    let wav = fs::read(serial_path.with_file_name("sound.wav"))
        .map_err(|e| format!("captured sound: {e}"))?;
    let (samples, rate) = super::p15_desktop::pcm16(&wav)?;
    let found = quarters(&samples, rate);
    // 1.5 s of 740 Hz hold at least four whole quarter-second windows at
    // eighth-second steps, two seconds of 990 Hz at least six, one second
    // of 1200 Hz at least two; the mixed pair at least two quarter seconds
    // holding both tones, the 370 Hz voice (-6 dB, panned left so the left
    // channel keeps it) at half the 1480 Hz one's level whatever the
    // guest's master volume.
    let (shot, streamed, resampled) = (near(&found, 740), near(&found, 990), near(&found, 1200));
    let mixed = mixed_at_half(&samples, rate);
    if shot < 4 || streamed < 6 || resampled < 2 || mixed < 2 {
        return Err(format!(
            "captured sound holds {shot} quarter seconds of 740 Hz (one-shot), {streamed} of 990 Hz (streaming), {resampled} of 1200 Hz (frequency) and {mixed} of 370 Hz with 1480 Hz (mixed); frequencies {found:?}"
        ));
    }
    details["captured"] = json!({"bytes":wav.len(),"quarter_seconds_hz":found,"one_shot_740":shot,"streaming_990":streamed,"frequency_1200":resampled,"mixed_370_1480":mixed});
    // Recording, from QEMU's silent input (never run on a reference PC).
    let (capture, capture_record) =
        session(run, true, "dsprobe.exe capture", "DSCAP-A", &DSCAP_STEPS)?;
    run.check("p15-dsound-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "capture":{"run":capture,"session":capture_record},
        "scope":"An MSVC-built Windows console program using dsound.dll (enumeration, primary format, secondary buffers with notifications, streaming, volume/pan, frequency, cursors, capture) in the guest, its output captured by QEMU's wav backend"}));
    Ok(())
}

const MIDIPROBE: Probe = Probe {
    name: "midiprobe",
    directory: "tests/fixtures/windows-sound",
    libraries: &["kernel32.lib", "WinMM.Lib"],
    modules: &["kernel32.dll", "winmm.dll"],
};
/// Steps midiprobe reports, in order.
const MIDI_STEPS: [&str; 14] = [
    "caps",
    "mapper",
    "open",
    "notes",
    "sysex",
    "volume",
    "close",
    "stream-open",
    "stream-play",
    "stream-close",
    "mci-wave",
    "mci-sequencer",
    "mci-errors",
    "mixer",
];
/// P15 MIDI-A: `midiprobe.exe` (MSVC, SDK import libraries, no C runtime)
/// uses the rest of Windows Multimedia from Terminal: the synthesizer
/// through midiOut (440 Hz, a square lead, one second), a MIDI stream
/// (659 Hz, a saw lead, two beats at 120 bpm), MCI playing a wave file
/// (587 Hz, one second) and a MIDI file through the sequencer (1047 Hz,
/// one second), and the mixer: the master volume set to 70% and back,
/// which the shell adopts as the user's volume. QEMU's wav backend captures
/// the four tones.
pub(super) fn verify_midi(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/midi-a", run.id))?;
    let fixture = build(run, &target, &MIDIPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-midi",
        &["windows/winmm.dll"],
    )?;
    let disk = super::win32::fresh_disk(run, "midi-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        let device =
            super::p15_desktop::line_after(run, &record, 0, "DOOR_P15 TRAY AUDIO present=")?;
        if !device.contains("present=1") {
            return Err(format!("no sound device in the guest: {device}"));
        }
        thread::sleep(Duration::from_secs(2));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(1));
        let before = serial(&run.root, &record)?.len();
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "midiprobe.exe",
            "MIDI-A",
            &MIDI_STEPS,
        )?;
        // The mixer's 70% reached door.audio and the shell adopted it,
        // then the restored volume likewise.
        let set = super::p15_desktop::line_after(
            run,
            &record,
            before,
            "DOOR_P15 AUDIO OUTPUT volume=70",
        )?;
        let adopted = super::p15_desktop::line_after(
            run,
            &record,
            before,
            "DOOR_P15 TRAY AUDIO ADOPTED volume=70",
        )?;
        let frame = scene::capture_settled(run, &mut c, "midi-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "midi-a-terminal")?;
        Ok(
            json!({"probe":probe,"device":device,"mixer_set":set,"shell_adopted":adopted,"screenshot":shot}),
        )
    })();
    let mut details = super::win32::quit_on_error(run, &record, result, "midi-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    let serial_path = run
        .root
        .join(record["serial"].as_str().ok_or("session serial")?);
    let wav = fs::read(serial_path.with_file_name("sound.wav"))
        .map_err(|e| format!("captured sound: {e}"))?;
    let (samples, rate) = super::p15_desktop::pcm16(&wav)?;
    let found = quarters(&samples, rate);
    // One second each: at least three whole quarter seconds at eighth-second
    // steps of every tone (attack and release take the edges).
    let tones = [
        ("midiout_440", near(&found, 440)),
        ("stream_659", near(&found, 659)),
        ("mci_wave_587", near(&found, 587)),
        ("mci_sequencer_1047", near(&found, 1047)),
    ];
    if tones.iter().any(|(_, n)| *n < 3) {
        return Err(format!(
            "captured sound: {tones:?} quarter seconds; frequencies {found:?}"
        ));
    }
    details["captured"] = json!({"bytes":wav.len(),"quarter_seconds_hz":found,
        "midiout_440":tones[0].1,"stream_659":tones[1].1,"mci_wave_587":tones[2].1,"mci_sequencer_1047":tones[3].1});
    run.check("p15-midi-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "scope":"An MSVC-built Windows console program using winmm.dll's MIDI synthesizer (midiOut, sysex, midiStream with callbacks and positions), MCI command strings for a wave file and a MIDI file, and the mixer's master volume, its tones captured by QEMU's wav backend"}));
    Ok(())
}

const WASAPIPROBE: Probe = Probe {
    name: "wasapiprobe",
    directory: "tests/fixtures/windows-sound",
    libraries: &["kernel32.lib", "Ole32.Lib", "WinMM.Lib"],
    modules: &["kernel32.dll", "ole32.dll", "winmm.dll"],
};
/// Steps wasapiprobe reports, in order.
const WASAPI_STEPS: [&str; 16] = [
    "enumerator",
    "default",
    "collection",
    "properties",
    "mix",
    "formats",
    "initialize",
    "render",
    "clock",
    "run",
    "event",
    "session",
    "endpoint",
    "notify",
    "capture",
    "mci-record",
];
/// P15 WASAPI-A (M4): `wasapiprobe.exe guest` uses the Core Audio APIs
/// through COM (ole32 activates MMDevApi.dll's MMDeviceEnumerator from its
/// registration): the default endpoints and their properties, IAudioClient
/// in shared mode against what Windows 11 answered on the owner's PC
/// (`wasapiprobe.exe reference`), rendering, the clock, event-driven
/// streams, session and endpoint volume, notifications and capture from
/// QEMU's silent input.
pub(super) fn verify_wasapi(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/wasapi-a", run.id))?;
    let fixture = build(run, &target, &WASAPIPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-wasapi",
        &[
            "windows/mmdevapi.dll",
            "windows/ole32.dll",
            "windows/winmm.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "wasapi-a")?;
    let silent = crate::emulator::silent_input();
    let launched = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION);
    drop(silent);
    let mut record = launched?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        let device =
            super::p15_desktop::line_after(run, &record, 0, "DOOR_P15 TRAY AUDIO present=")?;
        if !device.contains("present=1") {
            return Err(format!("no sound device in the guest: {device}"));
        }
        thread::sleep(Duration::from_secs(2));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(1));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "wasapiprobe.exe guest",
            "WASAPI-A",
            &WASAPI_STEPS,
        )?;
        // The microphone level the probe set is door.audio's, and the shell
        // kept it as the Sound page's Input volume.
        let input =
            super::p15_desktop::line_after(run, &record, 0, "DOOR_P15 AUDIO INPUT volume=60")?;
        let adopted = super::p15_desktop::line_after(
            run,
            &record,
            0,
            "DOOR_P15 TRAY AUDIO INPUT ADOPTED volume=60",
        )?;
        let frame = scene::capture_settled(run, &mut c, "wasapi-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "wasapi-a-terminal")?;
        Ok(json!({"probe":probe,"device":device,"input":input,"adopted":adopted,"screenshot":shot}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "wasapi-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p15-wasapi-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): wasapiprobe.exe reference 15/15 (renders silence, reads volumes, never opens the microphone)",
        "scope":"WASAPI-A: an MSVC-built probe using MMDeviceEnumerator, IMMDevice/IMMEndpoint/IPropertyStore, IAudioClient in shared mode (mix format, periods, format support, buffer sizes, state errors), IAudioRenderClient, IAudioClock, event-driven streams, ISimpleAudioVolume, IAudioEndpointVolume, endpoint notifications and IAudioCaptureClient from QEMU's silent input"}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quarters_find_each_tone() {
        let rate = 48_000u32;
        let mut wav = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0".to_vec();
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 4).to_le_bytes());
        wav.extend([4, 0, 16, 0]);
        wav.extend(b"data\0\0\0\0");
        // Half a second of silence, two of 660 Hz, a quarter of silence,
        // one of 880 Hz.
        let mut push = |hz: u32, seconds: f64| {
            for i in 0..(f64::from(rate) * seconds) as u32 {
                let v = if hz == 0 {
                    0.0
                } else {
                    (f64::from(i) * f64::from(hz) / f64::from(rate) * std::f64::consts::TAU).sin()
                        * 8000.0
                };
                wav.extend((v as i16).to_le_bytes());
                wav.extend((v as i16).to_le_bytes());
            }
        };
        push(0, 0.5);
        push(660, 2.0);
        push(0, 0.25);
        push(880, 1.0);
        let (samples, rate) = super::super::p15_desktop::pcm16(&wav).unwrap();
        let found = quarters(&samples, rate);
        assert!(near(&found, 660) >= 6, "{found:?}");
        assert!(near(&found, 880) >= 2, "{found:?}");
        assert_eq!(
            found.len(),
            near(&found, 660) + near(&found, 880),
            "{found:?}"
        );
    }
    #[test]
    fn mixed_voices_keep_their_six_decibels() {
        let rate = 48_000u32;
        let tone = |hz: f64, level: f64, i: usize| {
            level * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()
        };
        let mix = |low: f64| -> Vec<i32> {
            (0..rate as usize)
                .map(|i| (tone(370.0, low, i) + tone(1480.0, 4000.0, i)) as i32)
                .collect()
        };
        assert!(mixed_at_half(&mix(2000.0), rate) >= 6, "-6 dB is found");
        assert_eq!(mixed_at_half(&mix(4000.0), rate), 0, "equal levels are not");
        assert_eq!(mixed_at_half(&mix(0.0), rate), 0, "a missing voice is not");
    }

    #[test]
    fn goertzel_finds_mixed_tones_and_bursts() {
        let rate = 48_000u32;
        let sine = |hz: f64, i: usize| {
            (i as f64 * hz / f64::from(rate) * std::f64::consts::TAU).sin() * 4000.0
        };
        let mut samples: Vec<i32> = (0..rate as usize)
            .map(|i| (sine(440.0, i) + sine(1000.0, i)) as i32)
            .collect();
        samples.extend((0..rate as usize / 2).map(|_| 0));
        assert!(windows_with(&samples, rate, 0.25, &[440, 1000]) >= 6);
        assert_eq!(windows_with(&samples, rate, 0.25, &[550]), 0);
        let burst: Vec<i32> = (0..rate as usize / 4)
            .map(|i| sine(550.0, i) as i32)
            .collect();
        let mut gaps = burst.clone();
        gaps.extend(vec![0; rate as usize / 2]);
        gaps.extend(&burst);
        assert!(windows_with(&gaps, rate, 0.125, &[550]) >= 2);
        assert_eq!(windows_with(&gaps, rate, 0.125, &[440, 1000]), 0);
    }
}
