//! Windows wildcard matching as cmd.exe's file commands apply it: case
//! insensitive, `*` any run, `?` one character (or none at the end of the
//! name or before its extension dot), `*.*` everything, `name.*` also the
//! name without an extension, and a trailing `.` names without one.
use alloc::vec::Vec;

pub fn matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "*.*" {
        return true;
    }
    if let Some(stem) = pattern.strip_suffix(".*")
        && !name.contains('.')
        && glob(stem, name)
    {
        return true;
    }
    if let Some(stem) = pattern.strip_suffix('.')
        && !stem.ends_with('.')
    {
        return !name.contains('.') && glob(stem, name);
    }
    glob(pattern, name)
}
fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let n: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    // dp[j] = pattern prefix i matches name prefix j.
    let mut dp = alloc::vec![false; n.len() + 1];
    dp[0] = true;
    for &pc in &p {
        let mut next = alloc::vec![false; n.len() + 1];
        match pc {
            '*' => {
                let mut seen = false;
                for j in 0..=n.len() {
                    seen |= dp[j];
                    next[j] = seen;
                }
            }
            '?' => {
                for j in 0..=n.len() {
                    if !dp[j] {
                        continue;
                    }
                    // One character, or none at the end / before a dot.
                    if j < n.len() && n[j] != '.' {
                        next[j + 1] = true;
                    } else if j == n.len() || (n[j] == '.' && !n[j + 1..].contains(&'.')) {
                        next[j] = true;
                    } else {
                        next[j + 1] = true;
                    }
                }
            }
            c => {
                for j in 0..n.len() {
                    if dp[j] && n[j] == c {
                        next[j + 1] = true;
                    }
                }
            }
        }
        dp = next;
    }
    dp[n.len()]
}
/// Apply a COPY/REN destination template (`*.bak`, `new?.txt`) to a source
/// name, as cmd.exe does.
pub fn template(template: &str, source: &str) -> alloc::string::String {
    use alloc::string::String;
    if !template.contains(['*', '?']) {
        return String::from(template);
    }
    let t: Vec<char> = template.chars().collect();
    let s: Vec<char> = source.chars().collect();
    let mut out = String::new();
    let (mut i, mut j) = (0, 0);
    while i < t.len() {
        match t[i] {
            '*' => {
                // Copy up to the source character matching the template's
                // next literal (usually '.'), or to the end.
                match t.get(i + 1) {
                    Some(&stop) => {
                        let found = s[j..]
                            .iter()
                            .rposition(|&c| c.eq_ignore_ascii_case(&stop))
                            .map(|p| p + j);
                        let end = found.unwrap_or(s.len());
                        out.extend(&s[j..end]);
                        j = end;
                    }
                    None => {
                        out.extend(&s[j..]);
                        j = s.len();
                    }
                }
                i += 1;
            }
            '?' => {
                if j < s.len() && s[j] != '.' {
                    out.push(s[j]);
                    j += 1;
                }
                i += 1;
            }
            '.' => {
                // Skip the source to its dot.
                if let Some(p) = s[j..].iter().position(|&c| c == '.') {
                    j += p + 1;
                } else {
                    j = s.len();
                }
                out.push('.');
                i += 1;
            }
            c => {
                out.push(c);
                if j < s.len() && s[j] != '.' {
                    j += 1;
                }
                i += 1;
            }
        }
    }
    while out.ends_with('.') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_wildcards() {
        assert!(matches("*.txt", "Notes.TXT"));
        assert!(!matches("*.txt", "notes.txt.bak"));
        assert!(matches("*.*", "README"));
        assert!(matches("*", ".git"));
        assert!(matches("a?.txt", "ab.txt"));
        assert!(matches("a?.txt", "a.txt"));
        assert!(!matches("a?.txt", "abc.txt"));
        assert!(matches("readme.*", "readme"));
        assert!(matches("*.", "Makefile"));
        assert!(!matches("*.", "a.c"));
        assert!(matches("d*", "Documents"));
        assert!(matches("*file*", "MyFile.doc"));
    }
    #[test]
    fn rename_templates() {
        assert_eq!(template("*.bak", "report.txt"), "report.bak");
        assert_eq!(template("*.txt", "a.b.c"), "a.b.txt");
        assert_eq!(template("new_*", "x.txt"), "new_.txt");
        assert_eq!(template("plain.txt", "x.doc"), "plain.txt");
    }
}
