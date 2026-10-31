//! What a file's first bytes and its name say, for the backends whose files are zips or compound
//! files of a kind of their own (documents, spreadsheets), which the zip backend would otherwise
//! show as archives: the files a zip starts with, what its first says it is, and an extension.

use std::path::Path;

/// A zip's first local header.
pub(super) const ZIP: &[u8] = b"PK\x03\x04";

/// A compound file's signature: Word 97-2003 documents', Excel 97-2003 workbooks', and protected
/// Office packages'.
pub(super) const COMPOUND: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Whether `path` ends in one of `extensions`, in any case.
pub(super) fn named(path: &str, extensions: &[&str]) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// What the zip starting with `head` says it is in its first file, `mimetype`, stored as it is, as
/// OpenDocument has it (OpenDocument 1.3 part 2, §3.3): as much of it as `head` holds.
pub(super) fn mimetype(head: &[u8]) -> Option<&[u8]> {
    let header = |at: usize| head.get(at..at.saturating_add(2)).map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    let size = head.get(18..22).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let data = 30usize.saturating_add(header(26)).saturating_add(header(28));
    let end = data.saturating_add(usize::try_from(size).unwrap_or(usize::MAX)).min(head.len());
    (head.starts_with(ZIP) && head.get(30..38) == Some(b"mimetype")).then(|| head.get(data..end)).flatten()
}

/// The names of the files a zip starts with, as far as its first bytes go: each local header's,
/// until one whose size comes after its data, which can't be stepped over.
pub(super) struct Names<'a>(pub(super) &'a [u8]);

impl<'a> Iterator for Names<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let at = self.0;
        if !at.starts_with(ZIP) {
            return None;
        }
        let u16_at = |i: usize| at.get(i..i + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])));
        let u32_at = |i: usize| at.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let (flags, size, name, extra) = (u16_at(6)?, u32_at(18)?, u16_at(26)?, u16_at(28)?);
        let name_end = 30usize.checked_add(name)?;
        let found = at.get(30..name_end)?;
        // With a data descriptor (bit 3 of the flags), the size in the header may be 0: the next
        // header isn't where it says.
        let next = if flags & 0x8 != 0 { None } else { name_end.checked_add(extra)?.checked_add(usize::try_from(size).ok()?) };
        self.0 = next.and_then(|n| at.get(n..)).unwrap_or_default();
        Some(found)
    }
}

/// The first 1024 bytes of a zip of `files`, by name and contents, in order, stored as they are.
#[cfg(test)]
pub(super) fn zip_head(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::{Cursor, Write};

    use ::zip::write::SimpleFileOptions;
    use ::zip::{CompressionMethod, ZipWriter};

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        zip.start_file(*name, SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        zip.write_all(data).unwrap();
    }
    let mut bytes = zip.finish().unwrap().into_inner();
    bytes.truncate(1024);
    bytes
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_zips_names_are_read_as_far_as_its_first_bytes_go() {
        let head = zip_head(&[("one", b"1"), ("two", &[b'2'; 1000]), ("three", b"3")]);
        assert_eq!(Names(&head).collect::<Vec<_>>(), [&b"one"[..], b"two"], "the third starts past the head");
        // A size given after the data (bit 3 of the flags): the next header can't be found.
        let mut described = zip_head(&[("first", b"data"), ("second", b"more")]);
        described[6] |= 0x8;
        assert_eq!(Names(&described).collect::<Vec<_>>(), [&b"first"[..]]);
        let cut = zip_head(&[("a-long-name", b"x")]);
        assert_eq!(Names(&cut[..35]).count(), 0, "a name cut short");
        assert_eq!(Names(b"PK\x03\x04").count(), 0);
    }

    #[test]
    fn a_zips_first_file_says_what_it_is_when_named_mimetype() {
        let head = zip_head(&[("mimetype", b"application/x-kind"), ("content.xml", b"<content/>")]);
        assert_eq!(mimetype(&head), Some(&b"application/x-kind"[..]), "its contents, not the next file's");
        assert_eq!(mimetype(&zip_head(&[("content.xml", b"<content/>"), ("mimetype", b"x")])), None, "first only");
        assert_eq!(mimetype(&head[..40]), Some(&b"ap"[..]), "as far as the head goes");
        assert_eq!(mimetype(b"mimetype application/x-kind"), None, "not a zip");
    }

    #[test]
    fn a_name_is_matched_by_its_extension_in_any_case() {
        assert!(named("a/b.DocX", &["docx"]) && named("b.tar.xlsx", &["csv", "xlsx"]));
        assert!(!named("docx", &["docx"]) && !named("a.docx.gz", &["docx"]) && !named("a.docx", &[]));
    }

    proptest! {
        /// Whatever a file starts with, reading what it says goes no further than its first bytes
        /// and never panics.
        #[test]
        fn any_first_bytes_are_safe(head in proptest::collection::vec(any::<u8>(), 0..1024), zip in any::<bool>()) {
            let head = if zip { [ZIP, &head].concat() } else { head };
            prop_assert!(Names(&head).count() <= head.len() / 30 + 1);
            prop_assert!(mimetype(&head).is_none_or(|m| m.len() <= head.len()));
        }
    }
}
