//! Explicitly implemented qualification levels. Unknown/future phase names fail
//! closed; ordering here is used only for cumulative, already implemented gates.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Phase {
    P00,
    P01,
    P02,
    P03,
    P04,
    P05,
    P06,
    P07,
    P08,
    P09,
    P10,
    P11,
    /// Native UI, text and accessibility. Its acceptance suites are declared in
    /// `scenarios::acceptance_suites`; unimplemented suites fail verification.
    P12,
    /// Windows GUI integration. Authorized in parallel with P12 (owner decision 6,
    /// 23 October 2026); its gate includes everything P12's gate includes.
    P13,
    /// Complete desktop and visual fidelity (owner decision, 26 October 2026,
    /// ADR 0020); its gate includes everything P13's gate includes.
    P14,
    /// Network, audio and media (owner request, 1 November 2026); its gate
    /// includes everything P14's gate includes. It cannot pass before P14.
    P15,
    /// COM, services and installers, with the shell, elevation and security
    /// work (owner request, 2 November 2026); its gate includes P15's.
    P16,
    /// 32-bit (x86) Windows applications under WOW64 (owner decisions of 1
    /// and 2 November 2026); its gate includes P16's.
    P17,
}

impl Phase {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "P00" => Ok(Self::P00),
            "P01" => Ok(Self::P01),
            "P02" => Ok(Self::P02),
            "P03" => Ok(Self::P03),
            "P04" => Ok(Self::P04),
            "P05" => Ok(Self::P05),
            "P06" => Ok(Self::P06),
            "P07" => Ok(Self::P07),
            "P08" => Ok(Self::P08),
            "P09" => Ok(Self::P09),
            "P10" => Ok(Self::P10),
            "P11" => Ok(Self::P11),
            "P12" => Ok(Self::P12),
            "P13" => Ok(Self::P13),
            "P14" => Ok(Self::P14),
            "P15" => Ok(Self::P15),
            "P16" => Ok(Self::P16),
            "P17" => Ok(Self::P17),
            _ => Err(format!(
                "qualification schema for {value} is not implemented"
            )),
        }
    }
}

/// Shared storage moved in P08. Historical evidence is checked against its own
/// original source paths, never silently rebound to today's replacement files.
pub fn storage_source(report: &serde_json::Value, historical: &str) -> String {
    let shared = report["certified_phase"]
        .as_str()
        .and_then(|name| Phase::parse(name).ok())
        .is_some_and(|phase| phase >= Phase::P08);
    if shared
        && (historical.starts_with("kernel/src/storage/")
            || historical.starts_with("kernel/src/persistence/"))
    {
        historical.replacen("kernel/src/", "fs/storage-core/src/", 1)
    } else {
        historical.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_and_noncanonical_phase_names_fail_closed() {
        for value in [
            "P18", "P99", "P8", "p08", " P08", "P08 ", "p12", "P12 ", "p14", "P14 ", "p15", "P16 ",
            "p17", "",
        ] {
            assert!(Phase::parse(value).is_err());
        }
        assert!(Phase::parse("P08").unwrap() > Phase::P07);
        assert!(Phase::parse("P00").unwrap() < Phase::P01);
        // P12 and P13 are explicit schemas; P13 orders after P12 so every
        // cumulative `>= P12` gate also applies to P13.
        assert!(Phase::parse("P12").unwrap() > Phase::P11);
        assert!(Phase::parse("P13").unwrap() > Phase::P12);
        // P14 orders after P13 so every cumulative `>= P13` gate applies to it.
        assert!(Phase::parse("P14").unwrap() > Phase::P13);
        // P15 and P16 follow, each inheriting every earlier cumulative gate.
        assert!(Phase::parse("P15").unwrap() > Phase::P14);
        assert!(Phase::parse("P16").unwrap() > Phase::P15);
        assert!(Phase::parse("P17").unwrap() > Phase::P16);
    }

    #[test]
    fn moved_storage_sources_bind_to_the_certified_phase() {
        let old = "kernel/src/storage/ext2.rs";
        for phase in ["P06", "P07"] {
            assert_eq!(
                storage_source(&serde_json::json!({"certified_phase":phase}), old),
                old
            );
        }
        assert_eq!(
            storage_source(&serde_json::json!({"certified_phase":"P08"}), old),
            "fs/storage-core/src/storage/ext2.rs"
        );
        assert_eq!(
            storage_source(
                &serde_json::json!({"certified_phase":"P08"}),
                "kernel/src/block_device.rs"
            ),
            "kernel/src/block_device.rs"
        );
    }
}
