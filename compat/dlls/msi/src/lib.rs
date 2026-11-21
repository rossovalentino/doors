#![no_std]
//! Original msi.dll: Windows Installer's API. Databases opened read-only
//! with views (`sql`), records and summary information; sessions
//! (MsiOpenPackage) with properties, conditions and actions for custom
//! action DLLs; product queries over the Installer's records
//! (MsiQueryProductState, MsiGetProductInfo, MsiEnumProducts,
//! MsiEnumRelatedProducts, MsiGetComponentPath); and installation
//! (MsiInstallProduct, MsiConfigureProduct(Ex), MsiReinstallProduct) by
//! door-msi's engine in the calling process, with the internal UI (none,
//! basic progress, the package's own dialogs) or an external UI handler.
//! msiexec.exe is a thin program over `DoorMsiexecMain`.
//!
//! This module is the host-testable part: the SQL subset, product
//! information names and the UI's layout arithmetic.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
pub mod layout;
pub mod sql;

/// MsiGetProductInfo property names and the record value each reads.
pub fn product_info_value(property: &str) -> Option<&'static str> {
    Some(match property {
        "InstalledProductName" | "ProductName" => "ProductName",
        "VersionString" => "VersionString",
        "Version" => "Version",
        "Publisher" => "Publisher",
        "LocalPackage" => "LocalPackage",
        "InstallSource" => "InstallSource",
        "InstallDate" => "InstallDate",
        "PackageCode" => "PackageCode",
        "Language" => "Language",
        "AssignmentType" => "Assignment",
        "UpgradeCode" => "UpgradeCode",
        _ => return None,
    })
}

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

#[cfg(test)]
mod tests {
    extern crate std;
    use super::sql::{self, Field};
    use alloc::{string::ToString, vec};
    use door_msi::{Database, fixtures};

    fn db() -> alloc::vec::Vec<u8> {
        fixtures::tools(
            1,
            &fixtures::Payload {
                tool: b"MZ".to_vec(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn views_select_filter_join_and_order() {
        let bytes = db();
        let db = Database::open(&bytes).unwrap();
        let q =
            sql::parse("SELECT `Value` FROM `Property` WHERE `Property` = 'ProductName'").unwrap();
        let (cols, rows) = sql::run(&db, &q, &[]).unwrap();
        assert_eq!(cols[0].name, "Value");
        assert_eq!(cols[0].kind, "l0");
        assert_eq!(rows, vec![vec![Field::Str("Doors Tools".to_string())]]);
        // Parameters, ORDER BY, integers.
        let q = sql::parse(
            "SELECT `File`, `FileSize` FROM `File` WHERE `Sequence` > ? ORDER BY `Sequence`",
        )
        .unwrap();
        assert_eq!(q.parameters, 1);
        let (_, rows) = sql::run(&db, &q, &[Field::Int(2)]).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], Field::Str("fil_settings".to_string()));
        // A join with qualified columns.
        let q = sql::parse(
            "SELECT `Extension`.`Extension`, `Verb`.`Argument` FROM `Extension`, `Verb` WHERE `Extension`.`Extension` = `Verb`.`Extension_`",
        )
        .unwrap();
        let (_, rows) = sql::run(&db, &q, &[]).unwrap();
        assert_eq!(
            rows,
            vec![vec![
                Field::Str("doorsnote".to_string()),
                Field::Str("open \"%1\"".to_string())
            ]]
        );
        // IS NULL, OR, parentheses, *, the catalog.
        let q = sql::parse(
            "SELECT * FROM Registry WHERE (Value IS NULL OR Name = 'Runs') AND Root = 2",
        )
        .unwrap();
        let (cols, rows) = sql::run(&db, &q, &[]).unwrap();
        assert_eq!(cols.len(), 6);
        assert_eq!(cols[3].kind, "L255");
        assert_eq!(rows.len(), 2);
        let q = sql::parse("SELECT `Name` FROM `_Tables` WHERE `Name` = 'Upgrade'").unwrap();
        assert_eq!(sql::run(&db, &q, &[]).unwrap().1.len(), 1);
        let q = sql::parse("SELECT `Name`, `Data` FROM `Binary` ORDER BY `Name`").unwrap();
        let (_, rows) = sql::run(&db, &q, &[]).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][1], Field::Stream("Binary.DoorsBanner".to_string()));
        // The package's dialogs as the UI reads them.
        let d = crate::layout::dialog(&db, "WelcomeEulaDlg").unwrap();
        assert!(d.modal());
        assert_eq!(
            (d.width, d.height, d.default.as_str()),
            (370, 270, "Install")
        );
        let install = d.control("Install").unwrap();
        assert_eq!(install.events[0].event, "DoAction");
        assert_eq!(install.events[1].event, "EndDialog");
        assert_eq!(install.conditions.len(), 2);
        let p = crate::layout::dialog(&db, "ProgressDlg").unwrap();
        assert!(!p.modal());
        assert_eq!(
            p.control("ProgressBar").unwrap().subscriptions[0].0,
            "SetProgress"
        );
        let styles = crate::layout::text_styles(&db);
        assert_eq!(
            styles
                .iter()
                .find(|s| s.name == "DoorsFontTitle")
                .unwrap()
                .bits,
            1
        );
        assert!(
            crate::layout::rtf_text(&d.control("LicenseText").unwrap().text)
                .starts_with("DOORS TOOLS LICENSE")
        );
        let q = sql::parse("SELECT DISTINCT `Component_` FROM `FeatureComponents`").unwrap();
        assert_eq!(sql::run(&db, &q, &[]).unwrap().1.len(), 6);
        // Errors.
        assert_eq!(sql::parse("SELECT FROM X"), Err(sql::BAD_QUERY));
        assert_eq!(sql::parse("DELETE FROM `File`"), Err(sql::BAD_QUERY));
        let q = sql::parse("SELECT `Nope` FROM `File`").unwrap();
        assert_eq!(sql::run(&db, &q, &[]).err(), Some(sql::INVALID_TABLE));
        let q = sql::parse("SELECT * FROM `Missing`").unwrap();
        assert_eq!(sql::run(&db, &q, &[]).err(), Some(sql::INVALID_TABLE));
    }
}
