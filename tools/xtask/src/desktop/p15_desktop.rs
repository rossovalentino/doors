//! P15 desktop: the network and sound shell surfaces and Settings pages on a
//! private disk, with what the guest played captured by QEMU's wav backend.
//! The Start button's tooltip; the Win+X quick-link menu; Quick settings'
//! live volume row (a press on the slider sets door.audio's master volume)
//! and its Sound output page; Settings > System > Sound, whose Test button
//! must reach the captured WAV as a 440 Hz tone (SOUND-A playback); Network
//! & internet with the adapter's DHCP lease (CONNECT-A address); Advanced
//! network settings. Points come from the shell and Settings layouts.
use super::*;
use door_window::taskbar::{Layout, Slot, TrayTarget};
use door_workspace::settings_catalog::{Command, points};
use door_workspace::settings_layout::Page;

const RESOLUTION: (u32, u32) = (1280, 720);
fn center(r: door_window::graphics::Rect) -> (i32, i32) {
    (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2)
}
/// The last serial line containing `needle` after `before`.
pub(super) fn line_after(
    run: &Run,
    record: &Value,
    before: usize,
    needle: &str,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = text
            .get(before.min(text.len())..)
            .unwrap_or("")
            .lines()
            .rfind(|l| l.contains(needle))
        {
            return Ok(line.to_owned());
        }
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err("desktop guest failed; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// Win+R, the command, Return; Settings must report the page.
pub(super) fn open_page(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    command: &str,
    page: u32,
) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["meta_l", "r"])?;
    thread::sleep(Duration::from_millis(300));
    type_line(c, command)?;
    line_after(run, record, before, &format!("SETTINGS PAGE id={page}"))?;
    thread::sleep(Duration::from_millis(800));
    Ok(())
}
/// The frequency of every loud (peak over 1000) half-second-aligned second
/// of a capture, by zero crossings of its first channel.
pub(super) fn tones(wav: &[u8]) -> Result<Vec<u32>, String> {
    let (samples, rate) = pcm16(wav)?;
    let second = rate as usize;
    let mut found = Vec::new();
    let mut start = 0;
    while start + second <= samples.len() {
        let window = &samples[start..start + second];
        if window.iter().map(|v| v.abs()).max().unwrap_or(0) > 1000 {
            let crossings = window.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count() as u32;
            found.push(crossings);
        }
        start += second / 2;
    }
    Ok(found)
}
/// The dominant frequency of the loudest second of a 16-bit PCM WAV, by
/// zero crossings of its first channel: (Hz, peak amplitude).
pub(super) fn tone(wav: &[u8]) -> Result<(u32, i32), String> {
    let (samples, rate) = pcm16(wav)?;
    tone_of(&samples, rate)
}
/// First-channel samples and rate of a 16-bit PCM WAV capture.
pub(super) fn pcm16(wav: &[u8]) -> Result<(Vec<i32>, u32), String> {
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("captured sound is not a WAV file".into());
    }
    let (mut at, mut format, mut data) = (12usize, None, None);
    while at + 8 <= wav.len() {
        let len = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        match &wav[at..at + 4] {
            b"fmt " if body + 16 <= wav.len() => {
                let channels = u16::from_le_bytes([wav[body + 2], wav[body + 3]]);
                let rate = u32::from_le_bytes(wav[body + 4..body + 8].try_into().unwrap());
                let bits = u16::from_le_bytes([wav[body + 14], wav[body + 15]]);
                format = Some((channels, rate, bits));
            }
            // QEMU patches the sizes when it closes the file; an unpatched
            // data chunk runs to the end.
            b"data" => {
                let end = if len == 0 {
                    wav.len()
                } else {
                    (body + len).min(wav.len())
                };
                data = Some(&wav[body..end]);
                break;
            }
            _ => (),
        }
        at = body + len + (len & 1);
    }
    let (Some((channels, rate, 16)), Some(data)) = (format, data) else {
        return Err("captured sound is not 16-bit PCM".into());
    };
    let frame = usize::from(channels) * 2;
    let samples: Vec<i32> = data
        .chunks_exact(frame)
        .map(|f| i32::from(i16::from_le_bytes([f[0], f[1]])))
        .collect();
    Ok((samples, rate))
}
fn tone_of(samples: &[i32], rate: u32) -> Result<(u32, i32), String> {
    // The loudest second, or all of a shorter capture of at least half a
    // second (frequency scaled to the window).
    let second = (rate as usize).min(samples.len());
    if second < rate as usize / 2 {
        return Err(format!(
            "captured sound holds {} frames, under half a second",
            samples.len()
        ));
    }
    let start = (0..=samples.len() - second)
        .step_by(second / 10)
        .max_by_key(|s| {
            samples[*s..*s + second]
                .iter()
                .map(|v| i64::from(v.abs()))
                .sum::<i64>()
        })
        .unwrap_or(0);
    let window = &samples[start..start + second];
    let peak = window.iter().map(|v| v.abs()).max().unwrap_or(0);
    let crossings = window.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count() as u64;
    Ok(((crossings * u64::from(rate) / second as u64) as u32, peak))
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = super::win32::fresh_disk(run, "p15-desktop")?;
    let resolution = format!("{}x{}", RESOLUTION.0, RESOLUTION.1);
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, &resolution)?;
    let layout = Layout::new(RESOLUTION.0, RESOLUTION.1);
    let client = super::administration::maximized_client(RESOLUTION);
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        let mut p = (640, 360);
        thread::sleep(Duration::from_secs(2));
        let audio = line_after(run, &record, 0, "DOOR_P15 TRAY AUDIO present=")?;
        if !audio.contains("present=1") {
            return Err(format!("no sound device in the guest: {audio}"));
        }
        let lease = line_after(run, &record, 0, "DOOR_P15 NET LEASE ip=")?;
        line_after(run, &record, 0, "DOOR_P15 TRAY NETWORK Connected")?;
        // 1. The Start button's tooltip after a hover dwell.
        let (x, y) = center(layout.slot(Slot::Start, 6));
        move_to(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p15-start-tooltip", &[])?;
        // 2. Win+X: the quick-link menu.
        key(&mut c, &["meta_l", "x"])?;
        thread::sleep(Duration::from_millis(700));
        scene::capture_settled(run, &mut c, "p15-quick-link", &[])?;
        key(&mut c, &["esc"])?;
        // 3. Quick settings: the live volume row, a press at 80%, then the
        //    Sound output page.
        let (x, y) = center(layout.tray(TrayTarget::QuickSettings));
        let before = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, x, y)?;
        line_after(run, &record, before, "TRAY open=Some(Devices)")?;
        scene::capture_settled(run, &mut c, "p15-quick-settings", &[])?;
        let track = door_window::tray::volume_track(layout);
        let (tx, ty) = (track.x() + track.width() as i32 * 4 / 5, track.y() + 8);
        let before = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, tx, ty)?;
        let output = line_after(run, &record, before, "DOOR_P15 AUDIO OUTPUT volume=")?;
        let volume: u32 = output
            .split_once("volume=")
            .and_then(|(_, r)| r.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .ok_or("volume record")?;
        if !(75..=85).contains(&volume) {
            return Err(format!(
                "quick settings volume press set {volume}, not about 80"
            ));
        }
        scene::capture_settled(run, &mut c, "p15-quick-settings-volume", &[])?;
        let (x, y) = center(door_window::tray::output_button(layout));
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(500));
        scene::capture_settled(run, &mut c, "p15-sound-output", &[])?;
        key(&mut c, &["esc"])?;
        // 3b. The second tile page, Night light on (scanout warms the
        //     frame), the Accessibility page, Cast (Win+K), Project (Win+P).
        let (x, y) = center(layout.tray(TrayTarget::QuickSettings));
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(700));
        let (x, y) = center(door_window::tray::pages::page_down(layout));
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(700));
        scene::capture_settled(run, &mut c, "p15-quick-settings-page2", &[])?;
        let (nx, ny) = center(door_window::tray::tile(layout, 1));
        let before = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, nx, ny)?;
        let vision = line_after(run, &record, before, "DOOR_P15 QUICK VISION value=")?;
        if !vision.contains("value=101") {
            return Err(format!(
                "Night light tile did not turn night light on: {vision}"
            ));
        }
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p15-night-light", &[])?;
        click(&mut c, &mut p, nx, ny)?;
        thread::sleep(Duration::from_millis(1200));
        let (x, y) = center(door_window::tray::pages::page_up(layout));
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(700));
        let (x, y) = center(door_window::tray::tile(layout, 3));
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(600));
        scene::capture_settled(run, &mut c, "p15-quick-accessibility", &[])?;
        key(&mut c, &["esc"])?;
        thread::sleep(Duration::from_millis(500));
        key(&mut c, &["meta_l", "k"])?;
        thread::sleep(Duration::from_millis(900));
        scene::capture_settled_moving(run, &mut c, "p15-quick-cast", 20)?;
        key(&mut c, &["esc"])?;
        thread::sleep(Duration::from_millis(500));
        key(&mut c, &["meta_l", "p"])?;
        thread::sleep(Duration::from_millis(900));
        scene::capture_settled(run, &mut c, "p15-quick-project", &[])?;
        key(&mut c, &["esc"])?;
        // 4. Settings > System > Sound, maximized; Test plays the tone.
        open_page(run, &record, &mut c, "ms-settings:sound", 33)?;
        key(&mut c, &["meta_l", "up"])?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p15-settings-sound", &[])?;
        let (x, y) = points::command(Page::Sound, Command::TestSound, client.0, client.1)
            .ok_or("Sound > Speakers > Test")?;
        let before = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, x, y)?;
        line_after(run, &record, before, "DOOR_P15 AUDIO TEST_TONE id=")?;
        thread::sleep(Duration::from_millis(2000));
        // 4b. Preview plays FLAC through door.audio: Run "preview", Ctrl+O
        //     the installed chime, Space plays it to the end.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "preview")?;
        // Keys sent before Preview's window exists reach no window: wait
        // for it (a fixed delay was too short under load).
        line_after(run, &record, before, "title=Preview")?;
        thread::sleep(Duration::from_millis(1500));
        key(&mut c, &["ctrl", "o"])?;
        thread::sleep(Duration::from_millis(500));
        type_line(&mut c, "/system/share/doors-chime.flac")?;
        line_after(run, &record, before, "DOOR_PREVIEW OPEN object=")?;
        thread::sleep(Duration::from_millis(1000));
        key(&mut c, &["spc"])?;
        line_after(
            run,
            &record,
            before,
            "DOOR_P15 PREVIEW PLAY rate=48000 channels=2",
        )?;
        thread::sleep(Duration::from_millis(1200));
        scene::capture(run, &mut c, "p15-preview-playing", &[])?;
        thread::sleep(Duration::from_millis(2500));
        scene::capture_settled(run, &mut c, "p15-preview-audio", &[])?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(800));
        // 4c. Weather over TLS 1.3 (Open-Meteo, through the guest's NAT):
        //     search a city, the place and its forecast arrive.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "msnweather:")?;
        line_after(run, &record, before, "DOOR_INBOX READY app=weather")?;
        thread::sleep(Duration::from_millis(1200));
        key(&mut c, &["meta_l", "up"])?;
        thread::sleep(Duration::from_millis(900));
        click(&mut c, &mut p, RESOLUTION.0 as i32 - 156, 57)?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "Milan")?;
        let place = line_after(run, &record, before, "DOOR_P15 WEATHER PLACE").map_err(|e| {
            format!("Weather found no place (does the host have internet access?): {e}")
        })?;
        let forecast = line_after(run, &record, before, "DOOR_P15 WEATHER FORECAST")?;
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p15-weather", &[])?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(800));
        // 5. Network & internet > Ethernet: the adapter card, open, with the
        // lease.
        open_page(
            run,
            &record,
            &mut c,
            "ms-settings:network-ethernet",
            Page::Ethernet as u32,
        )?;
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p15-settings-ethernet", &[])?;
        // 6. Advanced network settings (ncpa.cpl).
        open_page(run, &record, &mut c, "ncpa.cpl", 20)?;
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p15-network-advanced", &[])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(40),
        )?;
        Ok(
            json!({"lease": lease, "quick_settings_volume": volume, "weather_place": place, "weather_forecast": forecast}),
        )
    })();
    let mut summary = super::win32::quit_on_error(run, &record, result, "p15-desktop-failure")?;
    seal_serial(&run.root, &mut record)?;
    // SOUND-A playback: the Test tone reached the device as 440 Hz.
    let serial_path = run
        .root
        .join(record["serial"].as_str().ok_or("session serial")?);
    let wav = fs::read(serial_path.with_file_name("sound.wav"))
        .map_err(|e| format!("captured sound: {e}"))?;
    let (hz, peak) = tone(&wav)?;
    let heard = tones(&wav)?;
    let near = |target: u32| heard.iter().any(|f| f.abs_diff(target) <= target / 30);
    if !near(440) || peak < 1000 {
        return Err(format!(
            "captured sound holds {heard:?} Hz, not the 440 Hz test tone"
        ));
    }
    if !near(660) {
        return Err(format!(
            "captured sound holds {heard:?} Hz, not Preview's 660 Hz chime"
        ));
    }
    summary["tones_hz"] = json!(heard);
    summary["tone_hz"] = json!(hz);
    summary["tone_peak"] = json!(peak);
    summary["sound_bytes"] = json!(wav.len());
    run.check(
        "p15-desktop-network-sound",
        json!({"session":record,"observed":summary,"scope":"Start tooltip, Win+X quick links, quick settings live volume and Sound output page, second tile page with Night light, Accessibility, Cast and Project pages, Settings Sound Test tone captured by the wav backend as 440 Hz, Preview playing the installed FLAC chime (660 Hz) through door.audio, Weather fetching a live Open-Meteo forecast over TLS 1.3, Network & internet Ethernet lease, Advanced network settings; owner volume not used"}),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_tone_analysis_finds_440_hz_in_a_stereo_capture() {
        let rate = 48_000u32;
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend(16u32.to_le_bytes());
        wav.extend([1, 0, 2, 0]);
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 4).to_le_bytes());
        wav.extend([4, 0, 16, 0]);
        wav.extend(b"data");
        wav.extend(0u32.to_le_bytes());
        for i in 0..rate * 2 {
            // Half a second of silence, then the tone.
            let v = if i < rate / 2 {
                0
            } else {
                let phase = (i as f64) * 440.0 * 2.0 * core::f64::consts::PI / f64::from(rate);
                (phase.sin() * 9000.0) as i16
            };
            wav.extend(v.to_le_bytes());
            wav.extend(v.to_le_bytes());
        }
        let (hz, peak) = tone(&wav).unwrap();
        assert!((439..=441).contains(&hz), "{hz}");
        assert!(peak > 8900);
        assert!(tone(b"nope").is_err());
    }
}
#[cfg(test)]
mod tone_list_tests {
    use super::*;
    #[test]
    fn tones_lists_each_loud_second() {
        let rate = 8000u32;
        let mut pcm = Vec::new();
        for (hz, seconds) in [(440.0, 2.0), (0.0, 1.0), (660.0, 2.0)] {
            for i in 0..(rate as f64 * seconds) as usize {
                let v = if hz == 0.0 {
                    0.0
                } else {
                    (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin()
                };
                let s = (v * 8000.0) as i16;
                pcm.extend_from_slice(&s.to_le_bytes());
                pcm.extend_from_slice(&s.to_le_bytes());
            }
        }
        let mut wav = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0".to_vec();
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&[4, 0, 16, 0]);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(&pcm);
        let heard = tones(&wav).unwrap();
        assert!(heard.iter().any(|f| f.abs_diff(440) <= 14), "{heard:?}");
        assert!(heard.iter().any(|f| f.abs_diff(660) <= 22), "{heard:?}");
    }
}
