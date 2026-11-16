//! HTML cut into its pieces, as the importer of the documents' backend cuts it: text, start tags
//! with their attributes, end tags, and what is none of them. For the passes of clv's own over HTML
//! before it is imported: what is a tag here is one there, and what holds no tag there (a comment,
//! a `<script>`'s text) holds none here.
//!
//! Nothing here builds a tree, decodes an entity or allocates: a piece is a slice of the text, and
//! a pass goes through them once.

use std::ops::Range;

/// The elements whose text the importer leaves out, tags and all, up to their end tag.
pub(super) const SKIPPED: [&str; 5] = ["script", "style", "noscript", "template", "textarea"];

/// A piece of HTML, as the importer cuts it.
pub(super) enum Piece<'h> {
    /// Text, its entities not yet decoded.
    Text(&'h str),
    /// A start tag.
    Start(StartTag<'h>),
    /// An end tag, by its name as written.
    End(&'h str),
    /// A comment, a doctype or a processing instruction.
    Other,
}

/// A start tag: its name as written, and of its attributes the first `class`, the first `lang` and
/// the first `id`, as written too, their entities not decoded. The rest of them, one by one
/// ([`StartTag::attributes`]).
pub(super) struct StartTag<'h> {
    pub(super) name: &'h str,
    pub(super) class: Option<&'h str>,
    pub(super) lang: Option<&'h str>,
    pub(super) id: Option<&'h str>,
    /// Ended by `/>`.
    pub(super) self_closed: bool,
    /// The tag as written, from its `<` to its `>`, or to the text's end where none ends it.
    text: &'h str,
}

impl<'h> StartTag<'h> {
    /// Whether it is a `name` element, in any case.
    pub(super) fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }

    /// Its attributes, in order, each as written, with its place in the tag.
    #[cfg_attr(not(any(feature = "epub", test)), expect(dead_code, reason = "only a book's chapters are written again tag by tag"))]
    pub(super) fn attributes(&self) -> impl Iterator<Item = Attribute<'h>> + use<'h> {
        Attributes::of(self.text, self.name.len().saturating_add(1))
    }
}

/// An attribute of a start tag: its name and its value as written, the value without its quotes,
/// its entities not decoded, and empty for an attribute without one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Attribute<'h> {
    pub(super) name: &'h str,
    pub(super) value: &'h str,
    /// Where it is in its tag's text, from the tag's `<`: its name's first byte, to past its value
    /// and the quote that closes it.
    pub(super) at: Range<usize>,
}

/// The attributes of a start tag, read one by one from its text.
struct Attributes<'h> {
    /// The text the tag starts, a `<` and its name first.
    rest: &'h str,
    /// Where the next attribute is looked for; once done, past the tag.
    at: usize,
    self_closed: bool,
    done: bool,
}

impl<'h> Attributes<'h> {
    /// Those of the tag `rest` starts with, whose name ends at `at`.
    fn of(rest: &'h str, at: usize) -> Self {
        Self { rest, at, self_closed: false, done: false }
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.rest.as_bytes().get(at).copied()
    }

    /// Past the blank space at `at`.
    fn blank(&self, mut at: usize) -> usize {
        while self.byte(at).is_some_and(|b| b.is_ascii_whitespace()) {
            at = at.saturating_add(1);
        }
        at
    }
}

impl<'h> Iterator for Attributes<'h> {
    type Item = Attribute<'h>;

    fn next(&mut self) -> Option<Attribute<'h>> {
        if self.done {
            return None;
        }
        let mut i = self.at;
        let found = loop {
            i = self.blank(i);
            match (self.byte(i), self.byte(i.saturating_add(1))) {
                (None, _) => break None,
                (Some(b'>'), _) => {
                    i = i.saturating_add(1);
                    break None;
                }
                (Some(b'/'), Some(b'>')) => {
                    self.self_closed = true;
                    i = i.saturating_add(2);
                    break None;
                }
                (Some(b'/'), _) => {
                    i = i.saturating_add(1);
                    continue;
                }
                _ => {}
            }
            let start = i;
            while self.byte(i).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'>' | b'/')) {
                i = i.saturating_add(1);
            }
            let name = self.rest.get(start..i).unwrap_or_default();
            if name.is_empty() {
                i = i.saturating_add(1);
                continue;
            }
            let mut end = i;
            i = self.blank(i);
            let mut value = "";
            if self.byte(i) == Some(b'=') {
                i = self.blank(i.saturating_add(1));
                match self.byte(i) {
                    Some(quote @ (b'"' | b'\'')) => {
                        let from = i.saturating_add(1);
                        let to = found(self.rest, from, if quote == b'"' { "\"" } else { "'" }).unwrap_or(self.rest.len());
                        value = self.rest.get(from..to).unwrap_or_default();
                        i = to.saturating_add(1);
                    }
                    _ => {
                        let from = i;
                        while self.byte(i).is_some_and(|b| !b.is_ascii_whitespace() && b != b'>') {
                            i = i.saturating_add(1);
                        }
                        value = self.rest.get(from..i).unwrap_or_default();
                    }
                }
                end = i.min(self.rest.len());
            }
            break Some(Attribute { name, value, at: start..end });
        };
        self.at = i;
        self.done = found.is_none();
        found
    }
}

/// Whether `byte` may be in a tag's name.
fn in_name(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-')
}

/// Where `needle` is in `text` from `from` on, as a place in `text`.
fn found(text: &str, from: usize, needle: &str) -> Option<usize> {
    text.get(from..)?.find(needle).map(|at| from.saturating_add(at))
}

/// Past the blank space at `at` in `html`.
pub(super) fn past_blank(html: &str, at: usize) -> usize {
    let blank = html.as_bytes().get(at..).unwrap_or_default().iter().take_while(|b| b.is_ascii_whitespace()).count();
    at.saturating_add(blank)
}

impl<'h> Piece<'h> {
    /// The piece of `html` that starts at `at`, and where the next one starts; `None` at its end.
    pub(super) fn at(html: &'h str, at: usize) -> Option<(Self, usize)> {
        let rest = html.get(at..).filter(|rest| !rest.is_empty())?;
        let end = html.len();
        let past = |found: Option<usize>, length: usize| found.map_or(end, |found| found.saturating_add(length));
        if !rest.starts_with('<') {
            let next = found(html, at, "<").unwrap_or(end);
            return Some((Self::Text(html.get(at..next).unwrap_or_default()), next));
        }
        if rest.starts_with("<!-->") {
            return Some((Self::Other, at.saturating_add(5)));
        }
        if rest.starts_with("<!--->") {
            return Some((Self::Other, at.saturating_add(6)));
        }
        if rest.starts_with("<!--") {
            return Some((Self::Other, past(found(html, at.saturating_add(4), "-->"), 3)));
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            return Some((Self::Other, past(found(html, at, ">"), 1)));
        }
        if let Some(name) = rest.strip_prefix("</") {
            let length = name.bytes().take_while(in_name).count();
            let Some(name) = name.get(..length).filter(|name| !name.is_empty()) else {
                return Some((Self::Text("</"), at.saturating_add(2)));
            };
            return Some((Self::End(name), past(found(html, at, ">"), 1)));
        }
        if !rest.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) {
            return Some((Self::Text("<"), at.saturating_add(1)));
        }
        let (tag, length) = start_tag(rest);
        Some((Self::Start(tag), at.saturating_add(length).min(end)))
    }
}

/// The start tag `rest` starts with, a `<` and a letter, and how long it is: to the `>` that ends it
/// outside its attributes' quotes, or all of `rest` where none does.
fn start_tag(rest: &str) -> (StartTag<'_>, usize) {
    let named = rest.as_bytes().get(1..).unwrap_or_default().iter().take_while(|b| in_name(b)).count();
    let name = rest.get(1..named.saturating_add(1)).unwrap_or_default();
    let mut attributes = Attributes::of(rest, named.saturating_add(1));
    let (mut class, mut lang, mut id) = (None, None, None);
    for attribute in attributes.by_ref() {
        let first = match attribute.name {
            name if name.eq_ignore_ascii_case("class") => &mut class,
            name if name.eq_ignore_ascii_case("lang") => &mut lang,
            name if name.eq_ignore_ascii_case("id") => &mut id,
            _ => continue,
        };
        first.get_or_insert(attribute.value);
    }
    let length = attributes.at.min(rest.len());
    let text = rest.get(..length).unwrap_or(rest);
    (StartTag { name, class, lang, id, self_closed: attributes.self_closed, text }, length)
}

/// Past the end tag of the `name` element whose text starts at `at` in `html`, tags in it no tags:
/// past the `>` of the first `</name` followed by blank space, a `>` or a `/`; the end of `html`
/// where there is none.
pub(super) fn past_end_tag(html: &str, at: usize, name: &str) -> usize {
    let bytes = html.as_bytes();
    let mut from = at;
    while let Some(rest) = bytes.get(from..).filter(|rest| !rest.is_empty()) {
        let ends = |window: &[u8]| window.strip_prefix(b"</").is_some_and(|after| after.eq_ignore_ascii_case(name.as_bytes()));
        let Some(start) = rest.windows(name.len().saturating_add(2)).position(ends) else { break };
        let after = from.saturating_add(start).saturating_add(name.len()).saturating_add(2);
        if bytes.get(after).is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/')) {
            return found(html, after, ">").map_or(html.len(), |end| end.saturating_add(1));
        }
        from = from.saturating_add(start).saturating_add(2);
    }
    html.len()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The first start tag of `html`.
    fn tag(html: &str) -> StartTag<'_> {
        match Piece::at(html, 0) {
            Some((Piece::Start(tag), _)) => tag,
            _ => panic!("no start tag: {html}"),
        }
    }

    /// Each attribute comes as written, in order, with where it is in its tag: quoted either way,
    /// unquoted, without a value, with blank space around its `=`, and twice.
    #[test]
    fn a_start_tags_attributes_come_as_written_with_their_places() {
        let html = r#"<a href="ch2.xhtml#s3" id = 'x' hidden class=note data-n=1 id="second">text"#;
        let tag = tag(html);
        let attributes: Vec<Attribute<'_>> = tag.attributes().collect();
        let written: Vec<(&str, &str, &str)> = attributes.iter().map(|a| (a.name, a.value, &html[a.at.clone()])).collect();
        assert_eq!(
            written,
            [
                ("href", "ch2.xhtml#s3", r#"href="ch2.xhtml#s3""#),
                ("id", "x", "id = 'x'"),
                ("hidden", "", "hidden"),
                ("class", "note", "class=note"),
                ("data-n", "1", "data-n=1"),
                ("id", "second", r#"id="second""#),
            ]
        );
        assert_eq!((tag.name, tag.id, tag.class, tag.self_closed), ("a", Some("x"), Some("note"), false), "the first of each, as before");
        assert_eq!(tag.attributes().count(), 6, "asked again, the same");
        // One ended short, and one never ended: as far as the text goes.
        let short = super::tests::tag(r#"<img src="a.png" alt=x/>after"#);
        assert_eq!(short.attributes().map(|a| (a.name, a.value)).collect::<Vec<_>>(), [("src", "a.png"), ("alt", "x/")]);
        let closed = super::tests::tag(r#"<br clear="all" />after"#);
        assert_eq!((closed.attributes().count(), closed.self_closed), (1, true));
        let open = super::tests::tag(r#"<p title="never closed"#);
        assert_eq!(open.attributes().map(|a| (a.name, a.value)).collect::<Vec<_>>(), [("title", "never closed")]);
        assert_eq!(super::tests::tag("<p>").attributes().count(), 0);
    }

    proptest! {
        /// Wherever HTML is cut and whatever it holds, its pieces follow one another to its end,
        /// and each attribute's place is within its tag and holds its name and its value.
        #[test]
        fn any_text_is_cut_into_pieces_and_attributes_within_it(html in r#"[<>/="' a-c!?-]{0,64}"#) {
            let mut at = 0;
            while let Some((piece, next)) = Piece::at(&html, at) {
                prop_assert!(next > at && next <= html.len());
                if let Piece::Start(tag) = piece {
                    let text = &html[at..next];
                    for attribute in tag.attributes() {
                        let written = text.get(attribute.at.clone());
                        prop_assert!(written.is_some_and(|w| w.starts_with(attribute.name) && w.contains(attribute.value)), "{text}: {attribute:?}");
                    }
                }
                at = next;
            }
            prop_assert_eq!(at, html.len());
        }
    }
}
