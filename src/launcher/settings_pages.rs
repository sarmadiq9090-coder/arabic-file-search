//! Windows settings pages (`ms-settings:` URIs) with Arabic and English words.

/// One settings page.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub uri: &'static str,
    pub title_ar: &'static str,
    pub title_en: &'static str,
    /// Section path shown under the title, per language.
    pub section_ar: &'static str,
    pub section_en: &'static str,
    /// Extra words in both languages ("wifi", "واي فاي", "انترنت").
    pub keywords: &'static [&'static str],
    /// Lowest Windows build that has this page (0 = all).
    pub min_build: u32,
}
