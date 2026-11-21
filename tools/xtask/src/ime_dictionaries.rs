//! `cargo xtask ime-dictionaries [--fetch] [--pin]`: acquire the openly
//! licensed IME dictionaries from their upstream URLs and convert them to
//! the door-ime dictionary format (`desktop/ime/src/dict.rs`):
//!
//! * Pinyin: CC-CEDICT readings (CC BY-SA 4.0) ranked by jieba word
//!   frequencies (MIT) -> `assets/ime/pinyin.dict` (CC BY-SA 4.0).
//! * Japanese: Mozc OSS dictionary and POS table (BSD-3-Clause, IPAdic-
//!   derived data) -> `assets/ime/japanese.dict`.
//!
//! Downloads go to `assets/ime/upstream/` and must match the SHA-256 pinned
//! in `assets/ime/sources.json`; `--pin` records the hashes of a first (or
//! deliberately refreshed) acquisition. Without `--fetch` only the
//! conversion runs, from files already present.
use crate::evidence::file_hash;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;
pub const SOURCES: &str = "assets/ime/sources.json";
/// Candidates kept per reading, readings kept by length.
const MAX_CANDIDATES: usize = 24;
const MAX_JAPANESE_READING: usize = 16;
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let fetch = args.iter().any(|a| a == "--fetch");
    let pin = args.iter().any(|a| a == "--pin");
    // Only the Japanese dictionary (its Mozc sources are pinned to a commit;
    // the CC-CEDICT export behind pinyin.dict is replaced daily upstream).
    let japanese_only = args.iter().any(|a| a == "--japanese");
    if args
        .iter()
        .any(|a| a != "--fetch" && a != "--pin" && a != "--japanese")
    {
        return Err("usage: cargo xtask ime-dictionaries [--fetch] [--pin] [--japanese]".into());
    }
    let manifest_path = root.join(SOURCES);
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| format!("{SOURCES}: {e}"))?)
            .map_err(|e| format!("{SOURCES}: {e}"))?;
    let upstream = root.join("assets/ime/upstream");
    fs::create_dir_all(&upstream).map_err(|e| e.to_string())?;
    let files = manifest["files"]
        .as_array_mut()
        .ok_or("sources.json: files")?;
    for f in files.iter_mut() {
        let name = f["name"].as_str().ok_or("file name")?.to_owned();
        let url = f["url"].as_str().ok_or("file url")?.to_owned();
        let path = upstream.join(&name);
        if japanese_only && !name.starts_with("mozc-") {
            continue;
        }
        // A download already matching its pin is kept.
        let pinned = f["sha256"].as_str().map(str::to_owned);
        if fetch && (pin || pinned.is_none() || file_hash(&path).ok() != pinned) {
            download(&url, &path)?;
        }
        let hash = file_hash(&path).map_err(|e| format!("{name}: {e} (run with --fetch)"))?;
        match f["sha256"].as_str() {
            Some(pinned) if pinned == hash => {}
            _ if pin => f["sha256"] = Value::String(hash.clone()),
            Some(pinned) => {
                return Err(format!(
                    "{name}: sha256 {hash} does not match the pinned {pinned}"
                ));
            }
            None => {
                return Err(format!(
                    "{name}: sha256 {hash} is not pinned (review the source, then --pin)"
                ));
            }
        }
        // The CC-CEDICT export is a zip holding cedict_ts.u8 (bsdtar reads zip).
        if name.ends_with(".zip") {
            let status = Command::new("tar")
                .arg("-xf")
                .arg(&path)
                .arg("-C")
                .arg(&upstream)
                .status()
                .map_err(|e| format!("tar: {e}"))?;
            if !status.success() {
                return Err(format!("{name}: extraction failed"));
            }
        }
    }
    if pin {
        let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
        fs::write(&manifest_path, text + "\n").map_err(|e| e.to_string())?;
    }
    let mut outputs = Vec::new();
    if !japanese_only {
        outputs.push(("pinyin.dict", pinyin(&upstream)?));
    }
    outputs.push(("japanese.dict", japanese(&upstream)?));
    for (name, text) in outputs {
        let out = root.join("assets/ime").join(name);
        fs::write(&out, &text).map_err(|e| e.to_string())?;
        println!(
            "assets/ime/{name}: {} bytes, sha256 {}",
            text.len(),
            file_hash(&out)?
        );
    }
    Ok(())
}
/// curl ships with Windows 10/11 and every supported host.
fn download(url: &str, to: &Path) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("{url}: only https sources"));
    }
    let partial = to.with_extension("partial");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--output",
        ])
        .arg(&partial)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        return Err(format!("{url}: download failed ({status})"));
    }
    fs::rename(&partial, to).map_err(|e| e.to_string())
}
fn read_text(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", path.display()))
}
fn header(sources: &[&str], licence: &str) -> String {
    let mut s = String::from("# door-ime dictionary v1\n");
    for src in sources {
        s.push_str(&format!("# source: {src}\n"));
    }
    s.push_str(&format!("# licence: {licence}\n"));
    s
}
fn usable(text: &str) -> bool {
    !text.is_empty()
        && !text.contains(['\t', ',', '\n', '\r'])
        && !text.chars().any(char::is_control)
}
/// Toneless pinyin key ("ni'hao") of a CC-CEDICT reading, if every
/// syllable is a plain pinyin syllable.
pub fn pinyin_key(reading: &str) -> Option<String> {
    let mut out = Vec::new();
    for s in reading.split(' ') {
        let s = s.to_ascii_lowercase().replace("u:", "v");
        let s = s.trim_end_matches(|c: char| c.is_ascii_digit());
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_lowercase()) {
            return None;
        }
        out.push(s.to_owned());
    }
    Some(out.join("'"))
}
fn pinyin(upstream: &Path) -> Result<String, String> {
    let mut freq: HashMap<String, u64> = HashMap::new();
    for line in read_text(&upstream.join("jieba-dict.txt"))?.lines() {
        let mut p = line.split(' ');
        if let (Some(w), Some(f)) = (p.next(), p.next().and_then(|f| f.parse().ok())) {
            freq.insert(w.to_owned(), f);
        }
    }
    let mut by_key: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
    for line in read_text(&upstream.join("cedict_ts.u8"))?.lines() {
        if line.starts_with('#') {
            continue;
        }
        let Some((head, rest)) = line.split_once(" [") else {
            continue;
        };
        let Some((reading, _)) = rest.split_once(']') else {
            continue;
        };
        let Some(simplified) = head.split(' ').nth(1) else {
            continue;
        };
        let Some(key) = pinyin_key(reading) else {
            continue;
        };
        if !usable(simplified) || simplified.chars().count() != key.split('\'').count() {
            continue;
        }
        let list = by_key.entry(key).or_default();
        if !list.iter().any(|c| c.1 == simplified) {
            list.push((
                freq.get(simplified).copied().unwrap_or(0),
                simplified.to_owned(),
            ));
        }
    }
    let mut out = header(
        &["CC-CEDICT (MDBG)", "jieba dict.txt frequencies"],
        "CC BY-SA 4.0 (CC-CEDICT); frequencies MIT (jieba)",
    );
    for (key, mut list) in by_key {
        list.sort_by(|a, b| b.0.cmp(&a.0));
        list.truncate(MAX_CANDIDATES);
        out.push_str(&key);
        for (_, w) in list {
            out.push('\t');
            out.push_str(&w);
        }
        out.push('\n');
    }
    Ok(out)
}
fn japanese(upstream: &Path) -> Result<String, String> {
    let mut pos: HashMap<String, String> = HashMap::new();
    for line in read_text(&upstream.join("mozc-id.def"))?.lines() {
        if let Some((id, p)) = line.split_once(' ') {
            pos.insert(id.to_owned(), p.to_owned());
        }
    }
    let mut by_reading: BTreeMap<String, Vec<(u32, String, bool)>> = BTreeMap::new();
    for n in 0..10 {
        for line in read_text(&upstream.join(format!("mozc-dictionary0{n}.txt")))?.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [reading, left, _, cost, surface] = f[..] else {
                continue;
            };
            let Ok(cost) = cost.parse::<u32>() else {
                continue;
            };
            let p = pos.get(left).map_or("", String::as_str);
            // Proper names make the file large and rarely help typing.
            if p.contains("固有名詞,人名")
                || reading.chars().count() > MAX_JAPANESE_READING
                || !usable(surface)
                || !usable(reading)
            {
                continue;
            }
            let particle = p.starts_with("助詞") || p.starts_with("助動詞");
            let list = by_reading.entry(reading.to_owned()).or_default();
            match list.iter_mut().find(|c| c.1 == surface) {
                Some(c) => c.0 = c.0.min(cost),
                None => list.push((cost, surface.to_owned(), particle)),
            }
        }
    }
    let mut out = header(
        &["Mozc OSS dictionary (dictionary_oss)"],
        "BSD-3-Clause (Mozc); IPAdic-derived data, IPA licence notice",
    );
    for (reading, mut list) in by_reading {
        list.sort();
        list.truncate(MAX_CANDIDATES);
        out.push_str(&reading);
        for (cost, surface, particle) in list {
            out.push_str(&format!(
                "\t{surface},{cost}{}",
                if particle { ",p" } else { "" }
            ));
        }
        out.push('\n');
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cedict_readings_become_toneless_keys() {
        assert_eq!(pinyin_key("ni3 hao3").as_deref(), Some("ni'hao"));
        assert_eq!(pinyin_key("lu:4 se4").as_deref(), Some("lv'se"));
        assert_eq!(pinyin_key("A A zhi4"), Some("a'a'zhi".into()));
        assert_eq!(pinyin_key("ka3 la1 O K"), Some("ka'la'o'k".into()));
        assert_eq!(pinyin_key("xx5 , xx"), None);
        assert!(usable("你好") && !usable("a,b") && !usable(""));
    }
}
