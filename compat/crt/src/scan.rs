//! The scanf family's engine (`__stdio_common_vsscanf` and its wide
//! twin): white space matching, literal units, `%%`, assignment
//! suppression (`*`), widths, length modifiers (`hh h l ll L j z t w I32
//! I64`), and `d i u o x X n c s [ ] p e E f g G a A`. Input comes from a
//! unit source; results go to the caller's pointers through `Store`.

use crate::number;

/// Where converted values go (the C pointers of the argument list).
pub trait Store {
    /// The next pointer argument.
    fn pointer(&mut self) -> usize;
    /// An integer of `size` bytes (1, 2, 4 or 8) at `pointer`.
    fn integer(&mut self, pointer: usize, size: usize, value: u64);
    /// A float (`double` when `wide`, else `float`) at `pointer`.
    fn float(&mut self, pointer: usize, double: bool, value: f64);
    /// Unit `index` of a string at `pointer` (narrow: a byte, wide: a
    /// UTF-16 unit); false when the buffer (`%s`'s secure size) is full.
    fn unit(&mut self, pointer: usize, index: usize, wide: bool, unit: u16) -> bool;
    /// The secure functions' buffer size argument (`_s` variants), if any.
    fn size(&mut self) -> Option<usize>;
}

/// The input: unit `i`, 0 at the end.
pub type Input<'a> = &'a dyn Fn(usize) -> u16;

/// Scan `input` with `format`: the count of assignments, or -1 (EOF) when
/// the input ended before the first conversion.
pub fn scan(
    format: &[u16],
    input: Input<'_>,
    store: &mut dyn Store,
    wide: bool,
    secure: bool,
) -> i32 {
    let mut pos = 0usize;
    let mut assigned = 0i32;
    let mut converted_any = false;
    let mut i = 0usize;
    let at = |i: usize| format.get(i).copied().unwrap_or(0);
    while i < format.len() && at(i) != 0 {
        let c = at(i);
        if number::is_space(c) {
            while number::is_space(input(pos)) {
                pos += 1;
            }
            i += 1;
            continue;
        }
        if c != u16::from(b'%') {
            if input(pos) != c {
                break;
            }
            pos += 1;
            i += 1;
            continue;
        }
        i += 1;
        if at(i) == u16::from(b'%') {
            while number::is_space(input(pos)) {
                pos += 1;
            }
            if input(pos) != u16::from(b'%') {
                break;
            }
            pos += 1;
            i += 1;
            continue;
        }
        let suppress = at(i) == u16::from(b'*');
        if suppress {
            i += 1;
        }
        let mut width = 0usize;
        while (0x30..=0x39).contains(&at(i)) {
            width = width * 10 + usize::from(at(i) - 0x30);
            i += 1;
        }
        let mut size = 4usize;
        let mut force_wide: Option<bool> = None;
        let mut double = false;
        loop {
            match at(i) {
                0x68 => {
                    size = if size == 2 { 1 } else { 2 };
                    force_wide = Some(false);
                }
                0x6C => {
                    if double || size == 8 {
                        size = 8;
                    } else if at(i + 1) == 0x6C {
                        size = 8;
                        i += 1;
                    } else {
                        double = true;
                        force_wide = Some(true);
                    }
                }
                0x4C => double = true,
                0x6A => size = 8,
                0x7A | 0x74 => size = core::mem::size_of::<usize>(),
                0x77 => force_wide = Some(true),
                0x49 => {
                    if at(i + 1) == 0x33 && at(i + 2) == 0x32 {
                        size = 4;
                        i += 2;
                    } else if at(i + 1) == 0x36 && at(i + 2) == 0x34 {
                        size = 8;
                        i += 2;
                    } else {
                        size = core::mem::size_of::<usize>();
                    }
                }
                _ => break,
            }
            i += 1;
        }
        // `%l` on integers is long (4 bytes on Windows); on floats double.
        let long_integer = double && size == 4;
        let conversion = at(i);
        i += 1;
        let Ok(conversion) = u8::try_from(conversion) else {
            break;
        };
        if !matches!(conversion, b'c' | b'[' | b'n') {
            while number::is_space(input(pos)) {
                pos += 1;
            }
        }
        if conversion != b'n' && input(pos) == 0 {
            if !converted_any {
                return -1;
            }
            break;
        }
        let limit = if width == 0 { usize::MAX } else { width };
        let bounded = |k: usize| if k < limit { input(pos + k) } else { 0 };
        match conversion {
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p' => {
                let base = match conversion {
                    b'd' | b'u' => 10,
                    b'i' => 0,
                    b'o' => 8,
                    _ => 16,
                };
                let Some(r) = number::integer(&bounded, base) else {
                    break;
                };
                if r.end == 0 {
                    break;
                }
                pos += r.end;
                converted_any = true;
                if !suppress {
                    let value = if r.negative {
                        r.magnitude.wrapping_neg()
                    } else {
                        r.magnitude
                    };
                    let size = if conversion == b'p' {
                        core::mem::size_of::<usize>()
                    } else if long_integer {
                        4
                    } else {
                        size
                    };
                    let p = store.pointer();
                    store.integer(p, size, value);
                    assigned += 1;
                }
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => {
                let (value, end, _) = number::float(&bounded);
                if end == 0 {
                    break;
                }
                pos += end;
                converted_any = true;
                if !suppress {
                    let p = store.pointer();
                    store.float(p, double, value);
                    assigned += 1;
                }
            }
            b'n' => {
                if !suppress {
                    let p = store.pointer();
                    store.integer(p, if long_integer { 4 } else { size }, pos as u64);
                }
            }
            b'c' | b'C' | b's' | b'S' | b'[' => {
                let string_wide = match force_wide {
                    Some(w) => w,
                    None => {
                        if wide {
                            !conversion.is_ascii_uppercase()
                        } else {
                            conversion.is_ascii_uppercase()
                        }
                    }
                };
                // The scan set of %[...].
                let mut set = [0u64; 4];
                let mut negate = false;
                if conversion == b'[' {
                    if at(i) == u16::from(b'^') {
                        negate = true;
                        i += 1;
                    }
                    let first = i;
                    while at(i) != 0 && (at(i) != u16::from(b']') || i == first) {
                        let a = at(i);
                        if at(i + 1) == u16::from(b'-')
                            && at(i + 2) != u16::from(b']')
                            && at(i + 2) != 0
                        {
                            let b = at(i + 2);
                            for u in a.min(b)..=a.max(b) {
                                if u < 256 {
                                    set[usize::from(u) / 64] |= 1 << (u % 64);
                                }
                            }
                            i += 3;
                        } else {
                            if a < 256 {
                                set[usize::from(a) / 64] |= 1 << (a % 64);
                            }
                            i += 1;
                        }
                    }
                    if at(i) != u16::from(b']') {
                        break;
                    }
                    i += 1;
                }
                let accepts = |u: u16| match conversion {
                    b'c' | b'C' => u != 0,
                    b's' | b'S' => u != 0 && !number::is_space(u),
                    _ => {
                        u != 0
                            && (u >= 256 && negate
                                || u < 256
                                    && ((set[usize::from(u) / 64] >> (u % 64)) & 1 == 1) != negate)
                    }
                };
                let count_limit = if matches!(conversion, b'c' | b'C') && width == 0 {
                    1
                } else {
                    limit
                };
                let pointer = if suppress { 0 } else { store.pointer() };
                let capacity = if secure && !suppress {
                    store.size()
                } else {
                    None
                };
                let mut n = 0usize;
                let mut overflow = false;
                while n < count_limit && accepts(input(pos + n)) {
                    if !suppress {
                        if capacity.is_some_and(|c| n + 1 > c) {
                            overflow = true;
                            break;
                        }
                        store.unit(pointer, n, string_wide, input(pos + n));
                    }
                    n += 1;
                }
                if n == 0 || overflow {
                    // Secure scanf writes an empty string on overflow.
                    if overflow && capacity.is_some_and(|c| c > 0) {
                        store.unit(pointer, 0, string_wide, 0);
                    }
                    break;
                }
                if !suppress && !matches!(conversion, b'c' | b'C') {
                    if capacity.is_some_and(|c| n + 1 > c) {
                        store.unit(pointer, 0, string_wide, 0);
                        break;
                    }
                    store.unit(pointer, n, string_wide, 0);
                }
                pos += n;
                converted_any = true;
                if !suppress {
                    assigned += 1;
                }
            }
            _ => break,
        }
    }
    assigned
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{collections::BTreeMap, vec::Vec};

    #[derive(Default)]
    struct Out {
        next: usize,
        ints: BTreeMap<usize, (usize, u64)>,
        floats: BTreeMap<usize, f64>,
        strings: BTreeMap<usize, Vec<u16>>,
        sizes: Vec<usize>,
    }
    impl Store for Out {
        fn pointer(&mut self) -> usize {
            self.next += 1;
            self.next
        }
        fn integer(&mut self, p: usize, size: usize, value: u64) {
            self.ints.insert(p, (size, value));
        }
        fn float(&mut self, p: usize, _: bool, value: f64) {
            self.floats.insert(p, value);
        }
        fn unit(&mut self, p: usize, index: usize, _: bool, unit: u16) -> bool {
            let s = self.strings.entry(p).or_default();
            if s.len() <= index {
                s.resize(index + 1, 0);
            }
            s[index] = unit;
            true
        }
        fn size(&mut self) -> Option<usize> {
            if self.sizes.is_empty() {
                None
            } else {
                Some(self.sizes.remove(0))
            }
        }
    }
    fn run(format: &str, input: &str, out: &mut Out) -> i32 {
        let f: Vec<u16> = format.bytes().map(u16::from).collect();
        let get = |i: usize| input.as_bytes().get(i).copied().map_or(0, u16::from);
        let secure = !out.sizes.is_empty();
        scan(&f, &get, out, false, secure)
    }
    fn text(out: &Out, p: usize) -> std::string::String {
        let s = &out.strings[&p];
        let end = s.iter().position(|&u| u == 0).unwrap_or(s.len());
        s[..end].iter().map(|&u| u as u8 as char).collect()
    }
    #[test]
    fn integers_strings_and_floats() {
        let mut o = Out::default();
        assert_eq!(run("%d %x %s %lf %c", "  -12 ff hello 2.5 Z", &mut o), 5);
        assert_eq!(o.ints[&1], (4, (-12i64) as u64));
        assert_eq!(o.ints[&2], (4, 255));
        assert_eq!(text(&o, 3), "hello");
        assert_eq!(o.floats[&4], 2.5);
        assert_eq!(o.strings[&5], [u16::from(b'Z')]);
    }
    #[test]
    fn widths_sets_suppression_and_counts() {
        let mut o = Out::default();
        assert_eq!(
            run("%3d%*d %[a-c]%n %[^,],%lld", "12345 abcx rest,9", &mut o),
            4
        );
        assert_eq!(o.ints[&1].1, 123);
        assert_eq!(text(&o, 2), "abc");
        assert_eq!(o.ints[&3].1, 9);
        assert_eq!(text(&o, 4), "x rest");
        assert_eq!(o.ints[&5], (8, 9));
    }
    #[test]
    fn eof_and_mismatch() {
        let mut o = Out::default();
        assert_eq!(run("%d", "   ", &mut o), -1);
        assert_eq!(run("%d", "x", &mut o), 0);
        assert_eq!(run("a%db", "a1c", &mut o), 1);
        assert_eq!(run("%i %i %i", "0x10 010 10", &mut Out::default()), 3);
    }
    #[test]
    fn secure_strings_are_bounded() {
        let mut o = Out {
            sizes: std::vec![4],
            ..Out::default()
        };
        assert_eq!(run("%s", "toolong", &mut o), 0);
        assert_eq!(o.strings[&1][0], 0);
        let mut o = Out {
            sizes: std::vec![8],
            ..Out::default()
        };
        assert_eq!(run("%s", "fits", &mut o), 1);
        assert_eq!(text(&o, 1), "fits");
    }
}
