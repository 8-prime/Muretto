//! Countries as the lobby names them (ISO 3166-1 alpha-2) and the flags the
//! game ships for them in `content/gui/NationFlags` (named by alpha-3).

use std::{collections::HashSet, path::PathBuf};

use crate::Install;

/// The game's flag pictures, by the files that are actually there.
#[derive(Debug, Clone, Default)]
pub struct Flags {
    dir: PathBuf,
    /// Upper-case alpha-3 codes with a picture.
    have: HashSet<String>,
}

impl Flags {
    pub fn scan(install: &Install) -> Self {
        let dir = install.root.join("content/gui/NationFlags");
        let have = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let is_png = path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
                let stem = path.file_stem()?.to_str()?.to_ascii_uppercase();
                is_png.then_some(stem)
            })
            .collect();
        Self { dir, have }
    }

    /// The flag for a two-letter country code, if the game has one.
    pub fn for_code(&self, alpha2: &str) -> Option<PathBuf> {
        let alpha3 = alpha3(alpha2)?;
        self.have
            .contains(alpha3)
            .then(|| self.dir.join(format!("{alpha3}.png")))
    }
}

/// The three-letter code for a two-letter one, in any case.
pub fn alpha3(alpha2: &str) -> Option<&'static str> {
    let code = alpha2.trim().to_ascii_uppercase();
    // Not ISO, but used for the United Kingdom often enough.
    let code = if code == "UK" { "GB" } else { code.as_str() };
    ALPHA3
        .binary_search_by_key(&code, |(two, _)| two)
        .ok()
        .map(|index| ALPHA3[index].1)
}

/// ISO 3166-1, sorted by the two-letter code.
const ALPHA3: &[(&str, &str)] = &[
    ("AD", "AND"),
    ("AE", "ARE"),
    ("AF", "AFG"),
    ("AG", "ATG"),
    ("AI", "AIA"),
    ("AL", "ALB"),
    ("AM", "ARM"),
    ("AO", "AGO"),
    ("AQ", "ATA"),
    ("AR", "ARG"),
    ("AS", "ASM"),
    ("AT", "AUT"),
    ("AU", "AUS"),
    ("AW", "ABW"),
    ("AX", "ALA"),
    ("AZ", "AZE"),
    ("BA", "BIH"),
    ("BB", "BRB"),
    ("BD", "BGD"),
    ("BE", "BEL"),
    ("BF", "BFA"),
    ("BG", "BGR"),
    ("BH", "BHR"),
    ("BI", "BDI"),
    ("BJ", "BEN"),
    ("BL", "BLM"),
    ("BM", "BMU"),
    ("BN", "BRN"),
    ("BO", "BOL"),
    ("BQ", "BES"),
    ("BR", "BRA"),
    ("BS", "BHS"),
    ("BT", "BTN"),
    ("BV", "BVT"),
    ("BW", "BWA"),
    ("BY", "BLR"),
    ("BZ", "BLZ"),
    ("CA", "CAN"),
    ("CC", "CCK"),
    ("CD", "COD"),
    ("CF", "CAF"),
    ("CG", "COG"),
    ("CH", "CHE"),
    ("CI", "CIV"),
    ("CK", "COK"),
    ("CL", "CHL"),
    ("CM", "CMR"),
    ("CN", "CHN"),
    ("CO", "COL"),
    ("CR", "CRI"),
    ("CU", "CUB"),
    ("CV", "CPV"),
    ("CW", "CUW"),
    ("CX", "CXR"),
    ("CY", "CYP"),
    ("CZ", "CZE"),
    ("DE", "DEU"),
    ("DJ", "DJI"),
    ("DK", "DNK"),
    ("DM", "DMA"),
    ("DO", "DOM"),
    ("DZ", "DZA"),
    ("EC", "ECU"),
    ("EE", "EST"),
    ("EG", "EGY"),
    ("EH", "ESH"),
    ("ER", "ERI"),
    ("ES", "ESP"),
    ("ET", "ETH"),
    ("FI", "FIN"),
    ("FJ", "FJI"),
    ("FK", "FLK"),
    ("FM", "FSM"),
    ("FO", "FRO"),
    ("FR", "FRA"),
    ("GA", "GAB"),
    ("GB", "GBR"),
    ("GD", "GRD"),
    ("GE", "GEO"),
    ("GF", "GUF"),
    ("GG", "GGY"),
    ("GH", "GHA"),
    ("GI", "GIB"),
    ("GL", "GRL"),
    ("GM", "GMB"),
    ("GN", "GIN"),
    ("GP", "GLP"),
    ("GQ", "GNQ"),
    ("GR", "GRC"),
    ("GS", "SGS"),
    ("GT", "GTM"),
    ("GU", "GUM"),
    ("GW", "GNB"),
    ("GY", "GUY"),
    ("HK", "HKG"),
    ("HM", "HMD"),
    ("HN", "HND"),
    ("HR", "HRV"),
    ("HT", "HTI"),
    ("HU", "HUN"),
    ("ID", "IDN"),
    ("IE", "IRL"),
    ("IL", "ISR"),
    ("IM", "IMN"),
    ("IN", "IND"),
    ("IO", "IOT"),
    ("IQ", "IRQ"),
    ("IR", "IRN"),
    ("IS", "ISL"),
    ("IT", "ITA"),
    ("JE", "JEY"),
    ("JM", "JAM"),
    ("JO", "JOR"),
    ("JP", "JPN"),
    ("KE", "KEN"),
    ("KG", "KGZ"),
    ("KH", "KHM"),
    ("KI", "KIR"),
    ("KM", "COM"),
    ("KN", "KNA"),
    ("KP", "PRK"),
    ("KR", "KOR"),
    ("KW", "KWT"),
    ("KY", "CYM"),
    ("KZ", "KAZ"),
    ("LA", "LAO"),
    ("LB", "LBN"),
    ("LC", "LCA"),
    ("LI", "LIE"),
    ("LK", "LKA"),
    ("LR", "LBR"),
    ("LS", "LSO"),
    ("LT", "LTU"),
    ("LU", "LUX"),
    ("LV", "LVA"),
    ("LY", "LBY"),
    ("MA", "MAR"),
    ("MC", "MCO"),
    ("MD", "MDA"),
    ("ME", "MNE"),
    ("MF", "MAF"),
    ("MG", "MDG"),
    ("MH", "MHL"),
    ("MK", "MKD"),
    ("ML", "MLI"),
    ("MM", "MMR"),
    ("MN", "MNG"),
    ("MO", "MAC"),
    ("MP", "MNP"),
    ("MQ", "MTQ"),
    ("MR", "MRT"),
    ("MS", "MSR"),
    ("MT", "MLT"),
    ("MU", "MUS"),
    ("MV", "MDV"),
    ("MW", "MWI"),
    ("MX", "MEX"),
    ("MY", "MYS"),
    ("MZ", "MOZ"),
    ("NA", "NAM"),
    ("NC", "NCL"),
    ("NE", "NER"),
    ("NF", "NFK"),
    ("NG", "NGA"),
    ("NI", "NIC"),
    ("NL", "NLD"),
    ("NO", "NOR"),
    ("NP", "NPL"),
    ("NR", "NRU"),
    ("NU", "NIU"),
    ("NZ", "NZL"),
    ("OM", "OMN"),
    ("PA", "PAN"),
    ("PE", "PER"),
    ("PF", "PYF"),
    ("PG", "PNG"),
    ("PH", "PHL"),
    ("PK", "PAK"),
    ("PL", "POL"),
    ("PM", "SPM"),
    ("PN", "PCN"),
    ("PR", "PRI"),
    ("PS", "PSE"),
    ("PT", "PRT"),
    ("PW", "PLW"),
    ("PY", "PRY"),
    ("QA", "QAT"),
    ("RE", "REU"),
    ("RO", "ROU"),
    ("RS", "SRB"),
    ("RU", "RUS"),
    ("RW", "RWA"),
    ("SA", "SAU"),
    ("SB", "SLB"),
    ("SC", "SYC"),
    ("SD", "SDN"),
    ("SE", "SWE"),
    ("SG", "SGP"),
    ("SH", "SHN"),
    ("SI", "SVN"),
    ("SJ", "SJM"),
    ("SK", "SVK"),
    ("SL", "SLE"),
    ("SM", "SMR"),
    ("SN", "SEN"),
    ("SO", "SOM"),
    ("SR", "SUR"),
    ("SS", "SSD"),
    ("ST", "STP"),
    ("SV", "SLV"),
    ("SX", "SXM"),
    ("SY", "SYR"),
    ("SZ", "SWZ"),
    ("TC", "TCA"),
    ("TD", "TCD"),
    ("TF", "ATF"),
    ("TG", "TGO"),
    ("TH", "THA"),
    ("TJ", "TJK"),
    ("TK", "TKL"),
    ("TL", "TLS"),
    ("TM", "TKM"),
    ("TN", "TUN"),
    ("TO", "TON"),
    ("TR", "TUR"),
    ("TT", "TTO"),
    ("TV", "TUV"),
    ("TW", "TWN"),
    ("TZ", "TZA"),
    ("UA", "UKR"),
    ("UG", "UGA"),
    ("UM", "UMI"),
    ("US", "USA"),
    ("UY", "URY"),
    ("UZ", "UZB"),
    ("VA", "VAT"),
    ("VC", "VCT"),
    ("VE", "VEN"),
    ("VG", "VGB"),
    ("VI", "VIR"),
    ("VN", "VNM"),
    ("VU", "VUT"),
    ("WF", "WLF"),
    ("WS", "WSM"),
    ("YE", "YEM"),
    ("YT", "MYT"),
    ("ZA", "ZAF"),
    ("ZM", "ZMB"),
    ("ZW", "ZWE"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_for_binary_search() {
        assert!(ALPHA3.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn maps_lobby_codes() {
        assert_eq!(alpha3("DE"), Some("DEU"));
        assert_eq!(alpha3("gb"), Some("GBR"));
        assert_eq!(alpha3("UK"), Some("GBR"));
        assert_eq!(alpha3("KR"), Some("KOR"));
        assert_eq!(alpha3("XX"), None);
    }

    #[test]
    fn finds_only_flags_that_exist() {
        let root = std::env::temp_dir().join(format!("muretto-flags-{}", std::process::id()));
        let dir = root.join("content/gui/NationFlags");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("DEU.png"), b"").unwrap();
        let flags = Flags::scan(&Install::at(root.clone(), None));
        assert_eq!(flags.for_code("DE"), Some(dir.join("DEU.png")));
        assert_eq!(flags.for_code("FR"), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
