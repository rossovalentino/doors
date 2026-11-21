//! Explorer item ordering: folders first, then the chosen column, with
//! Windows `StrCmpLogicalW`-style names ("file2" < "file10", case-insensitive;
//! `door_locale::collate`).
use core::cmp::Ordering;

/// Columns in Windows Details / "Sort by" order. The discriminant is the
/// display index; `persisted` keeps folder memory stable across versions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    /// Date modified (volume timestamps).
    Date,
    Type,
    Size,
}
impl SortKey {
    pub const ALL: [Self; 4] = [Self::Name, Self::Date, Self::Type, Self::Size];
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Date => "Date modified",
            Self::Type => "Type",
            Self::Size => "Size",
        }
    }
    /// Folder-memory value: Name 0, Type 1, Size 2 (as saved before Date
    /// modified existed), Date 3.
    pub fn persisted(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Type => 1,
            Self::Size => 2,
            Self::Date => 3,
        }
    }
    pub fn from_persisted(value: usize) -> Option<Self> {
        [Self::Name, Self::Type, Self::Size, Self::Date]
            .get(value)
            .copied()
    }
}

/// One comparable item: UTF-16 name, folder flag, logical size, type label
/// and modified time (FILETIME; 0 = not recorded).
#[derive(Clone, Copy)]
pub struct Item<'a> {
    pub name: &'a [u16],
    pub directory: bool,
    pub bytes: u64,
    pub kind: &'a str,
    pub modified: u64,
}

/// Explorer name order (total; see `door_locale::collate::compare_logical`).
pub fn natural(a: &[u16], b: &[u16]) -> Ordering {
    door_locale::collate::compare_logical(a, b)
}

/// Folders precede files in both directions; `descending` reverses only the
/// chosen column. Folders have no size, so Size orders them by name. Items
/// without a recorded date sort as oldest.
pub fn compare(key: SortKey, descending: bool, a: Item<'_>, b: Item<'_>) -> Ordering {
    b.directory.cmp(&a.directory).then_with(|| {
        let column = match key {
            SortKey::Name => Ordering::Equal,
            SortKey::Date => a.modified.cmp(&b.modified),
            SortKey::Type => a
                .kind
                .bytes()
                .map(|c| c.to_ascii_lowercase())
                .cmp(b.kind.bytes().map(|c| c.to_ascii_lowercase())),
            SortKey::Size if a.directory => Ordering::Equal,
            SortKey::Size => a.bytes.cmp(&b.bytes),
        };
        let order = column.then_with(|| natural(a.name, b.name));
        if descending { order.reverse() } else { order }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    #[test]
    fn names_are_case_insensitive_and_numbers_compare_by_value() {
        let mut names = ["file10", "File2", "file1", "a", "B", "file02", "file2"]
            .map(w)
            .to_vec();
        names.sort_by(|a, b| natural(a, b));
        let names: Vec<_> = names
            .iter()
            .map(|n| std::string::String::from_utf16(n).unwrap())
            .collect();
        assert_eq!(
            names,
            ["a", "B", "file1", "file02", "file2", "File2", "file10"]
        );
        assert_ne!(natural(&w("x"), &w("X")), Ordering::Equal);
        assert_eq!(
            natural(
                &w("v99999999999999999999999"),
                &w("v100000000000000000000000")
            ),
            Ordering::Less
        );
        assert_eq!(natural(&w("résumé 2"), &w("Resume 10")), Ordering::Less);
    }
    #[test]
    fn folders_lead_in_both_directions_and_columns_reverse() {
        let (f1, f2, a, b) = (w("Alpha"), w("Zed"), w("big.txt"), w("small.png"));
        let items = [
            Item {
                name: &f1,
                directory: true,
                bytes: 0,
                kind: "File folder",
                modified: 5,
            },
            Item {
                name: &f2,
                directory: true,
                bytes: 0,
                kind: "File folder",
                modified: 0,
            },
            Item {
                name: &a,
                directory: false,
                bytes: 900,
                kind: "Text Document",
                modified: 0,
            },
            Item {
                name: &b,
                directory: false,
                bytes: 10,
                kind: "PNG File",
                modified: 7,
            },
        ];
        for key in SortKey::ALL {
            for descending in [false, true] {
                let mut sorted = items.to_vec();
                sorted.sort_by(|x, y| compare(key, descending, *x, *y));
                assert!(sorted[0].directory && sorted[1].directory, "{key:?}");
            }
        }
        let mut by_size = items.to_vec();
        by_size.sort_by(|x, y| compare(SortKey::Size, true, *x, *y));
        assert_eq!(by_size[2].bytes, 900);
        let mut by_type = items.to_vec();
        by_type.sort_by(|x, y| compare(SortKey::Type, false, *x, *y));
        assert_eq!(by_type[2].kind, "PNG File");
        // Date modified: not recorded (0) is oldest; newest first descending.
        let mut by_date = items.to_vec();
        by_date.sort_by(|x, y| compare(SortKey::Date, false, *x, *y));
        assert_eq!(
            by_date.iter().map(|i| i.modified).collect::<Vec<_>>(),
            [0, 5, 0, 7]
        );
        by_date.sort_by(|x, y| compare(SortKey::Date, true, *x, *y));
        assert_eq!(
            by_date.iter().map(|i| i.modified).collect::<Vec<_>>(),
            [5, 0, 7, 0]
        );
    }
    #[test]
    fn persisted_sort_values_keep_their_meaning() {
        for key in SortKey::ALL {
            assert_eq!(SortKey::from_persisted(key.persisted()), Some(key));
        }
        assert_eq!(SortKey::from_persisted(1), Some(SortKey::Type));
        assert_eq!(SortKey::from_persisted(4), None);
        assert_eq!(SortKey::ALL.map(|k| k as usize), [0, 1, 2, 3]);
    }
}
