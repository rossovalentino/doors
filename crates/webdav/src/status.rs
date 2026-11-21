//! The sync host's report (`%LOCALAPPDATA%\NoneDrive\status`), read by the
//! NoneDrive page in Settings and by File Explorer's status column: the
//! overall state, the last complete sync, and the items that need the
//! user (skipped names, conflict copies, transfers that keep failing).
use alloc::{format, string::String, vec::Vec};

pub const HEADER: &str = "NoneDrive status 1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    UpToDate,
    Syncing,
    Paused,
    Offline,
    /// The server refused the user name or app password.
    SignIn,
    /// The server's certificate is untrusted (its SHA-256) and was never
    /// confirmed, or it changed since it was.
    Certificate([u8; 32]),
    /// Anything else, with the message the page shows.
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A server item left alone, and why (`sync::Skip` as text).
    Skipped { path: String, reason: String },
    /// A conflict: the server's version is at `path`, this PC's at `copy`.
    Conflict { path: String, copy: String },
    /// A transfer that failed this round (retried next time).
    Failed { path: String, reason: String },
    /// Being uploaded or downloaded now.
    Syncing { path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// FILETIME of the last complete sync (0: never).
    pub last_sync: u64,
    pub items: Vec<Item>,
}
/// Items listed at most (the page shows the first ones).
pub const MAX_ITEMS: usize = 200;

fn safe(s: &str) -> bool {
    !s.contains(['\t', '\n', '\r'])
}
impl Status {
    pub fn encode(&self) -> String {
        let state = match &self.state {
            State::UpToDate => String::from("uptodate"),
            State::Syncing => String::from("syncing"),
            State::Paused => String::from("paused"),
            State::Offline => String::from("offline"),
            State::SignIn => String::from("signin"),
            State::Certificate(f) => format!("certificate\t{}", crate::account::hex(f)),
            State::Error(m) if safe(m) => format!("error\t{m}"),
            State::Error(_) => String::from("error\t"),
        };
        let mut out = format!("{HEADER}\nstate\t{state}\nlast\t{}\n", self.last_sync);
        for item in self.items.iter().take(MAX_ITEMS) {
            let line = match item {
                Item::Skipped { path, reason } => ("skipped", path, reason.as_str()),
                Item::Conflict { path, copy } => ("conflict", path, copy.as_str()),
                Item::Failed { path, reason } => ("failed", path, reason.as_str()),
                Item::Syncing { path } => ("syncing", path, ""),
            };
            if safe(line.1) && safe(line.2) {
                out.push_str(&format!("{}\t{}\t{}\n", line.0, line.1, line.2));
            }
        }
        out
    }
    pub fn decode(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let mut f = lines.next()?.split('\t');
        if f.next()? != "state" {
            return None;
        }
        let state = match f.next()? {
            "uptodate" => State::UpToDate,
            "syncing" => State::Syncing,
            "paused" => State::Paused,
            "offline" => State::Offline,
            "signin" => State::SignIn,
            "certificate" => {
                let h = f.next()?;
                let mut fp = [0u8; 32];
                if h.len() != 64 {
                    return None;
                }
                for (i, b) in fp.iter_mut().enumerate() {
                    *b = u8::from_str_radix(h.get(i * 2..i * 2 + 2)?, 16).ok()?;
                }
                State::Certificate(fp)
            }
            "error" => State::Error(f.next().unwrap_or("").into()),
            _ => return None,
        };
        let last_sync = lines.next()?.strip_prefix("last\t")?.parse().ok()?;
        let mut items = Vec::new();
        for line in lines {
            let mut f = line.splitn(3, '\t');
            let (kind, path, extra) = (f.next()?, f.next()?, f.next().unwrap_or(""));
            let (path, extra) = (String::from(path), String::from(extra));
            items.push(match kind {
                "skipped" => Item::Skipped {
                    path,
                    reason: extra,
                },
                "conflict" => Item::Conflict { path, copy: extra },
                "failed" => Item::Failed {
                    path,
                    reason: extra,
                },
                "syncing" => Item::Syncing { path },
                _ => return None,
            });
        }
        Some(Self {
            state,
            last_sync,
            items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    #[test]
    fn the_status_round_trips() {
        let s = Status {
            state: State::Certificate([7; 32]),
            last_sync: 133_000_000_000_000_000,
            items: vec![
                Item::Skipped {
                    path: "a:b".into(),
                    reason: "name".into(),
                },
                Item::Conflict {
                    path: "r.docx".into(),
                    copy: "r-PC.docx".into(),
                },
                Item::Syncing {
                    path: "big.bin".into(),
                },
            ],
        };
        assert_eq!(Status::decode(&s.encode()), Some(s));
        let e = Status {
            state: State::Error("The server is full.".into()),
            last_sync: 0,
            items: vec![],
        };
        assert_eq!(Status::decode(&e.encode()), Some(e));
        assert_eq!(Status::decode("nope"), None);
    }
}
