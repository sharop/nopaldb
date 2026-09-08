// src/index/analyzer.rs
//
// Per-index configuration of the full-text analyzer (#74): which language
// the stemmer and the stop-word list speak, and whether accents are folded.
//
// This is plain data, compiled in every tier so that `IndexOptions`, NQL and
// the Python surface can name it; turning it into a tantivy `TextAnalyzer`
// lives in `fulltext.rs` behind the `fulltext` feature.

use serde::{Deserialize, Serialize};

use crate::error::{NopalError, Result};

/// How the text of one full-text index is analyzed, on the way in and on the
/// way out: the query is analyzed with the same chain as the documents.
///
/// `Default` is exactly what every index did before 0.5.13: tantivy's
/// `default` tokenizer (split on non-alphanumerics, drop tokens longer than
/// 40 chars, lowercase). No stemming, no stop words, accents kept, so
/// `clasificacion` does not find `clasificación`.
///
/// Changing the analyzer of an existing index is not supported in place: the
/// tokens on disk were produced by the old chain, and a query analyzed by the
/// new one would silently miss them. Drop the index and create it again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextAnalyzer {
    /// Language of the stemmer and the stop-word list, by tantivy's name in
    /// lowercase (`"spanish"`, `"english"`, …). See [`Self::LANGUAGES`].
    #[serde(default)]
    pub language: Option<String>,
    /// Reduce words to their stem (`catálogos` → `catalog`). Needs `language`.
    #[serde(default)]
    pub stemming: bool,
    /// Drop the language's stop words (`de`, `la`, `el`, …). Needs `language`
    /// and one of the languages with a list ([`Self::STOPWORD_LANGUAGES`]).
    #[serde(default)]
    pub stopwords: bool,
    /// Fold accents and other diacritics to ASCII (`clasificación` →
    /// `clasificacion`), so a query typed without accents still matches.
    #[serde(default)]
    pub ascii_folding: bool,
}

impl FullTextAnalyzer {
    /// Languages tantivy can stem (its `Language` enum, lowercased).
    pub const LANGUAGES: &'static [&'static str] = &[
        "arabic", "danish", "dutch", "english", "finnish", "french", "german", "greek", "hungarian", "italian",
        "norwegian", "portuguese", "romanian", "russian", "spanish", "swedish", "tamil", "turkish",
    ];

    /// Languages tantivy ships a stop-word list for.
    pub const STOPWORD_LANGUAGES: &'static [&'static str] = &[
        "danish", "dutch", "english", "finnish", "french", "german", "hungarian", "italian", "norwegian", "portuguese",
        "russian", "spanish", "swedish",
    ];

    /// Analyzer for a language with everything on: stemming, stop words and
    /// accent folding. What `create index … with (language = "spanish")` means.
    pub fn for_language(language: &str) -> Self {
        Self {
            language: Some(language.to_ascii_lowercase()),
            stemming: true,
            stopwords: true,
            ascii_folding: true,
        }
    }

    /// `true` for the analyzer every index had before 0.5.13.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Reject combinations tantivy cannot build, with the fix in the message.
    pub fn validate(&self) -> Result<()> {
        match &self.language {
            Some(lang) if !Self::LANGUAGES.contains(&lang.as_str()) => {
                return Err(NopalError::index_error(format!(
                    "full-text analyzer: unknown language `{lang}`; tantivy knows {}",
                    Self::LANGUAGES.join(", ")
                )));
            }
            Some(lang) if self.stopwords && !Self::STOPWORD_LANGUAGES.contains(&lang.as_str()) => {
                return Err(NopalError::index_error(format!(
                    "full-text analyzer: tantivy has no stop-word list for `{lang}` (it has {}); set stopwords = false",
                    Self::STOPWORD_LANGUAGES.join(", ")
                )));
            }
            None if self.stemming => {
                return Err(NopalError::index_error(
                    "full-text analyzer: stemming needs a language (e.g. language = \"spanish\")".to_string(),
                ));
            }
            None if self.stopwords => {
                return Err(NopalError::index_error(
                    "full-text analyzer: stopwords need a language (e.g. language = \"spanish\")".to_string(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    /// Name under which the chain is registered in the tantivy index. The
    /// default analyzer keeps tantivy's own `default`, so an index created
    /// before 0.5.13 opens with a byte-identical schema.
    pub fn tokenizer_name(&self) -> String {
        if self.is_default() {
            return "default".to_string();
        }
        let mut name = String::from("nopal");
        if let Some(lang) = &self.language {
            name.push('_');
            name.push_str(lang);
        }
        if self.stemming {
            name.push_str("_stem");
        }
        if self.stopwords {
            name.push_str("_stop");
        }
        if self.ascii_folding {
            name.push_str("_fold");
        }
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_tantivys_default() {
        let a = FullTextAnalyzer::default();
        assert!(a.is_default());
        assert_eq!(a.tokenizer_name(), "default");
        a.validate().unwrap();
    }

    #[test]
    fn for_language_turns_everything_on_and_names_it() {
        let a = FullTextAnalyzer::for_language("Spanish");
        assert_eq!(a.language.as_deref(), Some("spanish"));
        assert!(a.stemming && a.stopwords && a.ascii_folding);
        assert_eq!(a.tokenizer_name(), "nopal_spanish_stem_stop_fold");
        a.validate().unwrap();
    }

    #[test]
    fn validation_explains_the_fix() {
        let unknown = FullTextAnalyzer { language: Some("klingon".into()), ..Default::default() };
        assert!(unknown.validate().unwrap_err().to_string().contains("unknown language"));
        let no_list = FullTextAnalyzer { language: Some("tamil".into()), stopwords: true, ..Default::default() };
        assert!(no_list.validate().unwrap_err().to_string().contains("stopwords = false"));
        let stem_no_lang = FullTextAnalyzer { stemming: true, ..Default::default() };
        assert!(stem_no_lang.validate().unwrap_err().to_string().contains("needs a language"));
        let fold_only = FullTextAnalyzer { ascii_folding: true, ..Default::default() };
        fold_only.validate().unwrap();
        assert_eq!(fold_only.tokenizer_name(), "nopal_fold");
    }

    #[test]
    fn serde_round_trip_and_missing_fields_default() {
        let a = FullTextAnalyzer::for_language("french");
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<FullTextAnalyzer>(&json).unwrap(), a);
        let partial: FullTextAnalyzer = serde_json::from_str(r#"{"language":"english"}"#).unwrap();
        assert_eq!(partial.language.as_deref(), Some("english"));
        assert!(!partial.stemming && !partial.stopwords && !partial.ascii_folding);
    }
}
