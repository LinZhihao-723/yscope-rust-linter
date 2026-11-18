//! Symbol paths: the `::`-separated paths that doc links and `syn` paths name, normalized so that a
//! symbol is spelled the same on both sides.

use std::fmt;

/// A `::`-separated path of identifiers.
///
/// Doc text and `syn` paths are normalized the same way, so a symbol is spelled the same on both
/// sides.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolPath {
    segments: Vec<String>,
}

impl SymbolPath {
    /// Parses a path written in doc text, such as the code of an intra-doc link.
    ///
    /// The text is normalized as follows:
    ///
    /// * A rustdoc disambiguator such as `struct@` or `fn@` is dropped.
    /// * A trailing `()` or `!` is dropped.
    /// * `<T as Trait>::f` becomes `Trait::f`.
    /// * Generic arguments are dropped, so `Foo::<T>::bar` becomes `Foo::bar`.
    /// * A leading `::`, `crate`, `self` or `super` is dropped.
    ///
    /// # Returns
    ///
    /// The path, or [`None`] if the text isn't a path of identifiers.
    pub fn parse(text: &str) -> Option<Self> {
        let mut text = text.trim();
        if let Some((disambiguator, rest)) = text.split_once('@')
            && is_identifier(disambiguator)
        {
            text = rest;
        }
        let text = text.strip_suffix("()").unwrap_or(text);
        let text = text.strip_suffix('!').unwrap_or(text);
        let text = rewrite_qualified_self(text)?;
        let text = strip_generic_arguments(&text)?;
        Self::from_segments(text.split("::").map(str::trim))
    }

    /// Converts a `syn` path. Generic arguments are dropped, and so are a leading `::`, `crate`,
    /// `self` or `super`.
    ///
    /// # Returns
    ///
    /// The path, or [`None`] if nothing is left after normalization.
    pub fn from_syn_path(path: &syn::Path) -> Option<Self> {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        let leading_colon = path.leading_colon.is_some().then_some("");
        Self::from_segments(
            leading_colon
                .into_iter()
                .chain(segments.iter().map(String::as_str)),
        )
    }

    /// # Returns
    ///
    /// A one-segment path holding `name`.
    pub fn single(name: &str) -> Self {
        Self {
            segments: vec![name.strip_prefix("r#").unwrap_or(name).to_owned()],
        }
    }

    /// # Returns
    ///
    /// The `anyhow::Error` path.
    pub fn anyhow_error() -> Self {
        Self {
            segments: vec!["anyhow".to_owned(), "Error".to_owned()],
        }
    }

    /// # Returns
    ///
    /// The last segment.
    ///
    /// # Panics
    ///
    /// Panics if the path has no segment, which every factory function rules out.
    pub fn last(&self) -> &str {
        self.segments
            .last()
            .expect("a symbol path has at least one segment")
    }

    /// # Returns
    ///
    /// The second-to-last segment, or [`None`] for a one-segment path.
    pub fn qualifier(&self) -> Option<&str> {
        self.segments
            .len()
            .checked_sub(2)
            .map(|index| self.segments[index].as_str())
    }

    /// # Returns
    ///
    /// The number of segments.
    pub const fn len(&self) -> usize {
        self.segments.len()
    }

    /// # Returns
    ///
    /// The path without its last segment, or [`None`] for a one-segment path.
    pub fn parent(&self) -> Option<Self> {
        (self.segments.len() > 1).then(|| Self {
            segments: self.segments[..self.segments.len() - 1].to_vec(),
        })
    }

    /// Two type paths name the same type when their last segments are equal and, if both are
    /// qualified, their qualifiers are equal, so `io::Error` and `std::io::Error` match.
    ///
    /// # Returns
    ///
    /// Whether `self` and `other` name the same type.
    pub fn matches_type(&self, other: &Self) -> bool {
        self.last() == other.last()
            && match (self.qualifier(), other.qualifier()) {
                (Some(qualifier), Some(other_qualifier)) => qualifier == other_qualifier,
                _ => true,
            }
    }

    /// # Returns
    ///
    /// Every non-empty prefix of the path, from the shortest to the whole path.
    pub fn prefixes(&self) -> impl Iterator<Item = Self> + '_ {
        (1..=self.segments.len()).map(|len| Self {
            segments: self.segments[..len].to_vec(),
        })
    }

    /// Builds a path from raw segments, dropping a leading empty segment (from `::`) and any
    /// leading `crate`, `self` or `super` segments.
    ///
    /// # Type Parameters
    ///
    /// * `'segment` - The lifetime of the raw segments.
    ///
    /// # Returns
    ///
    /// The path, or [`None`] if no segment is left or a segment isn't an identifier.
    fn from_segments<'segment>(raw_segments: impl Iterator<Item = &'segment str>) -> Option<Self> {
        let mut segments = Vec::new();
        for (index, raw_segment) in raw_segments.enumerate() {
            if index == 0 && raw_segment.is_empty() {
                continue;
            }
            let segment = raw_segment.strip_prefix("r#").unwrap_or(raw_segment);
            if !is_identifier(segment) {
                return None;
            }
            if segments.is_empty() && matches!(segment, "crate" | "self" | "super") {
                continue;
            }
            segments.push(segment.to_owned());
        }
        (!segments.is_empty()).then_some(Self { segments })
    }
}

impl fmt::Display for SymbolPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.segments.join("::"))
    }
}

/// # Returns
///
/// Whether `name` is in upper camel case: it starts with an uppercase letter and has no underscore.
pub fn is_upper_camel_case(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase) && !name.contains('_')
}

/// # Returns
///
/// Whether `text` is a non-empty identifier.
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && chars.all(|rest| rest.is_alphanumeric() || rest == '_')
}

/// Rewrites a qualified-self path: `<T as Trait>::f` becomes `Trait::f`, and `<T>::f` becomes
/// `T::f`.
///
/// # Returns
///
/// The rewritten text, the text itself if it doesn't start with `<`, or [`None`] if the angle
/// brackets aren't balanced.
fn rewrite_qualified_self(text: &str) -> Option<String> {
    if !text.starts_with('<') {
        return Some(text.to_owned());
    }
    let mut depth = 0_usize;
    let mut close_index = None;
    for (index, character) in text.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    close_index = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let close_index = close_index?;
    let inner = &text[1..close_index];
    let rest = &text[close_index + 1..];
    let qualifier = inner
        .rsplit_once(" as ")
        .map_or(inner, |(_, trait_path)| trait_path)
        .trim();
    Some(format!("{qualifier}{rest}"))
}

/// Removes every balanced `<...>` group, together with the `::` of a turbofish before it.
///
/// # Returns
///
/// The text without generic arguments, or [`None`] if the angle brackets aren't balanced.
fn strip_generic_arguments(text: &str) -> Option<String> {
    let mut stripped = String::with_capacity(text.len());
    let mut depth = 0_usize;
    for character in text.chars() {
        match character {
            '<' => {
                if depth == 0
                    && let Some(without_turbofish) = stripped.strip_suffix("::")
                {
                    stripped.truncate(without_turbofish.len());
                }
                depth += 1;
            }
            '>' => depth = depth.checked_sub(1)?,
            _ if depth == 0 => stripped.push(character),
            _ => {}
        }
    }
    (depth == 0).then_some(stripped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> SymbolPath {
        SymbolPath::parse(text).expect("valid symbol path")
    }

    #[test]
    fn type_paths_match_by_last_segment_and_qualifier() {
        assert!(path("std::io::Error").matches_type(&path("io::Error")));
        assert!(path("Error").matches_type(&path("std::io::Error")));
        assert!(!path("fmt::Error").matches_type(&path("io::Error")));
        assert!(!path("io::Error").matches_type(&path("io::ErrorKind")));
    }

    #[test]
    fn doc_text_is_normalized() {
        let cases = [
            ("crate::job::Job::start", "job::Job::start"),
            ("super::super::cache::Cache", "cache::Cache"),
            ("self::helper()", "helper"),
            ("::std::fs::read", "std::fs::read"),
            ("Foo::<T>::bar", "Foo::bar"),
            ("Vec<u8>::new", "Vec::new"),
            (
                "<Server as YamlConfig>::from_yaml_file",
                "YamlConfig::from_yaml_file",
            ),
            ("<Server>::run", "Server::run"),
            ("anyhow::bail!", "anyhow::bail"),
            ("fn@spawn", "spawn"),
            ("r#type::new", "type::new"),
        ];
        for (text, expected) in cases {
            assert_eq!(path(text).to_string(), expected, "normalizing `{text}`");
        }
        for text in [
            "limit <= 0",
            "Err(e)",
            "",
            "https://example.com",
            "<Server as Trait",
        ] {
            assert_eq!(SymbolPath::parse(text), None, "parsing `{text}`");
        }
    }

    #[test]
    fn syn_paths_are_normalized() {
        let syn_path: syn::Path =
            syn::parse_str("crate::cache::Cache::<u64>::insert").expect("valid path");
        assert_eq!(
            SymbolPath::from_syn_path(&syn_path).map(|path| path.to_string()),
            Some("cache::Cache::insert".to_owned())
        );
    }
}
