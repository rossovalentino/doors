#![no_std]
#![forbid(unsafe_code)]
//! Door accessibility: the semantic tree every native control publishes
//! (role, name, value, states, actions, bounds, relationships), its bounded
//! versioned wire format (`wire`), assistive-client mirrors (`mirror`) and
//! Narrator-style captions (`caption`).
//!
//! The tree is a parallel semantic interface, never inferred from pixels
//! (master specification section 58). Providers publish a full tree once and
//! deltas afterwards; the window server's broker validates every message,
//! strips protected content, and serves it only to authorized assistive
//! clients of an unlocked session.
extern crate alloc;
pub mod caption;
pub mod mirror;
mod node;
mod tree;
pub mod wire;
pub use node::*;
pub use tree::*;

/// Text dump of a tree in reading order, one node per line, for stored
/// semantic-tree fixtures and inspectors. Stable across runs.
pub fn dump(tree: &Tree) -> alloc::string::String {
    use core::fmt::Write;
    let mut out = alloc::string::String::new();
    for (node, depth) in tree.walk() {
        let (x, y, w, h) = node.bounds.logical();
        let _ = write!(
            out,
            "{:width$}{} {:?}",
            "",
            node.role.spoken(),
            node.name,
            width = depth * 2
        );
        if !node.value.is_empty() {
            let _ = write!(out, " value={:?}", node.value);
        }
        let states: alloc::vec::Vec<_> = node.states.names().collect();
        if !states.is_empty() {
            let _ = write!(out, " [{}]", states.join(" "));
        }
        let actions: alloc::vec::Vec<_> = node.actions.names().collect();
        if !actions.is_empty() {
            let _ = write!(out, " {{{}}}", actions.join(" "));
        }
        if let Some(r) = node.range {
            let _ = write!(out, " range={}..{}@{}", r.min, r.max, r.now);
        }
        if let Some((p, n)) = node.position {
            let _ = write!(out, " pos={p}/{n}");
        }
        if node.level > 0 {
            let _ = write!(out, " level={}", node.level);
        }
        if let Some((a, c)) = node.selection {
            let _ = write!(out, " sel={a}..{c}");
        }
        if tree.focus == Some(node.id) {
            out.push_str(" *focus*");
        }
        let _ = writeln!(out, " @{x},{y} {w}x{h}");
    }
    out
}

/// `dump` without what varies with window size, display scale and focus:
/// geometry, the focus marker and the `FOCUSED` and `OFFSCREEN` states.
/// Guest assistive checks compare this form with stored host fixtures.
pub fn outline(tree: &Tree) -> alloc::string::String {
    let mut out = alloc::string::String::new();
    for line in dump(tree).lines() {
        out.push_str(&outline_line(line));
        out.push('\n');
    }
    out
}
/// One `dump` line in `outline` form.
pub fn outline_line(line: &str) -> alloc::string::String {
    let line = line.rsplit_once(" @").map_or(line, |(l, _)| l);
    let line = line.replace(" *focus*", "");
    let Some((head, rest)) = line.split_once(" [") else {
        return line;
    };
    let Some((states, tail)) = rest.split_once(']') else {
        return line;
    };
    let kept: alloc::vec::Vec<&str> = states
        .split(' ')
        .filter(|s| !matches!(*s, "FOCUSED" | "OFFSCREEN"))
        .collect();
    if kept.is_empty() {
        alloc::format!("{head}{tail}")
    } else {
        alloc::format!("{head} [{}]{tail}", kept.join(" "))
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
