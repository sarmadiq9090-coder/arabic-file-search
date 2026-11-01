//! Installed programs: Start menu shortcuts and Store apps.

/// One installed program.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    /// Display name as Windows shows it ("Google Chrome").
    pub name: String,
    /// `.lnk` path or `shell:AppsFolder\<AUMID>`.
    pub target: String,
    /// Extra names it should be found by (Arabic names, abbreviations).
    pub aliases: Vec<String>,
}
