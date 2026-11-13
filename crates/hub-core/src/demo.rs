//! The example data of a demo instance comes in two languages: the same projects, issues
//! and history, with everything a person reads written in English or in Polish.

/// The language the example data is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoLanguage {
    En,
    Pl,
}

impl DemoLanguage {
    /// `"en"` or `"pl"`; anything else is not a language of the example data.
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Self::En),
            "pl" => Some(Self::Pl),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
        }
    }
}

/// One text of the example data, in each of its languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemoText {
    pub en: &'static str,
    pub pl: &'static str,
}

impl DemoText {
    pub const EMPTY: Self = Self::same("");

    pub const fn new(en: &'static str, pl: &'static str) -> Self {
        Self { en, pl }
    }

    /// A text that reads the same in both languages: a product name, a tag such as "seo".
    pub const fn same(text: &'static str) -> Self {
        Self { en: text, pl: text }
    }

    pub const fn pick(&self, language: DemoLanguage) -> &'static str {
        match language {
            DemoLanguage::En => self.en,
            DemoLanguage::Pl => self.pl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_is_parsed_from_its_code() {
        assert_eq!(DemoLanguage::parse("en"), Some(DemoLanguage::En));
        assert_eq!(DemoLanguage::parse("pl"), Some(DemoLanguage::Pl));
        assert_eq!(DemoLanguage::parse("de"), None);
        assert_eq!(DemoLanguage::parse("EN"), None);
        assert_eq!(DemoLanguage::parse(DemoLanguage::Pl.code()), Some(DemoLanguage::Pl));
    }

    #[test]
    fn a_text_is_picked_by_language() {
        let text = DemoText::new("Password reset", "Reset hasła");

        assert_eq!(text.pick(DemoLanguage::En), "Password reset");
        assert_eq!(text.pick(DemoLanguage::Pl), "Reset hasła");
        assert_eq!(DemoText::same("seo").pick(DemoLanguage::En), DemoText::same("seo").pick(DemoLanguage::Pl));
    }
}
