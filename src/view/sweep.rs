//! Pictures printed inside tmux, deleted from the terminal once no pane holds them.
//!
//! Printed inside tmux, a page is an image in the terminal and placeholder cells in the pane
//! (`kitty.rs`), which tmux keeps as text. `view` exits with the image still in the terminal, so
//! that the cells show it again when they scroll back or their window comes back, and nothing
//! ever tells the terminal when tmux lets go of them: `clear-history`, a closed pane, the history
//! past its `history-limit`. kitty keeps such an image until its storage quota pushes it out
//! (about 400 MB of GPU memory a window) or its tab closes, the tmux server gone or not.
//!
//! So each page printed inside tmux is recorded, by pane and image id, in a user option of the
//! tmux server ([`OPTION`]), and every run inside tmux first reads each recorded pane: an image
//! whose cells it no longer holds, or whose pane is gone, is deleted (`a=d,d=I`), and its record
//! goes. A pane that can't be read keeps its records. Only recorded ids are deleted, never
//! another program's images; and a delete reaches the terminals showing this pane at the time,
//! so an image in a terminal attached to another session stays (`docs/architecture.md`).

use std::collections::{HashMap, HashSet};

use super::kitty::{self, Wrap};
use super::tmux;

/// The tmux server's user option the records are kept in.
const OPTION: &str = "@clv-printed";

/// The most pages recorded; past it the oldest records go (their images stay, as before).
/// Each is at most 28 bytes with the space after it, so all of them fit in what `tmux` reads back.
const MAX_RECORDED: usize = 2048;

/// The longest pane name a record keeps (tmux's are `%` and up to 10 digits).
const MAX_PANE: usize = 16;

/// A page printed inside tmux: the pane it went to, as tmux names it, and its image.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    pane: String,
    id: u32,
}

/// The records in `value` (`%3:402653191 %3:402653192`), oldest first: what isn't one is
/// dropped, and only the last [`MAX_RECORDED`] are kept. A pane is only a name here: one tmux
/// doesn't list is gone, and its image with it.
fn parse(value: &str) -> Vec<Record> {
    let records: Vec<Record> = value
        .split_whitespace()
        .filter_map(|entry| {
            let (pane, id) = entry.split_once(':')?;
            let id = id.parse::<u32>().ok().filter(|id| *id != 0)?;
            (!pane.is_empty() && pane.len() <= MAX_PANE).then(|| Record { pane: pane.to_owned(), id })
        })
        .collect();
    let extra = records.len().saturating_sub(MAX_RECORDED);
    records.into_iter().skip(extra).collect()
}

fn format(records: &[Record]) -> String {
    records.iter().map(|r| format!("{}:{}", r.pane, r.id)).collect::<Vec<_>>().join(" ")
}

/// Split `records` into those that stay and the ids to delete: a record goes when its pane is not
/// among `panes`, or its pane's cells (`shown`) no longer name its image. A pane without an
/// entry in `shown` couldn't be read, and keeps its records.
fn sweep(records: Vec<Record>, panes: &HashSet<String>, shown: &HashMap<String, HashSet<u32>>) -> (Vec<Record>, Vec<u32>) {
    let mut gone = Vec::new();
    let kept = records
        .into_iter()
        .filter(|record| {
            let held = panes.contains(&record.pane) && shown.get(&record.pane).is_none_or(|ids| ids.contains(&record.id));
            if !held {
                gone.push(record.id);
            }
            held
        })
        .collect();
    (kept, gone)
}

/// Delete the images printed inside tmux that no pane holds any more: their escape codes go into
/// `out`, to be written to the terminal through this pane. Best effort: when tmux can't be asked,
/// nothing is deleted.
pub(crate) fn forget(out: &mut Vec<u8>) {
    let Ok(value) = tmux::server_option(OPTION) else { return };
    let records = parse(&value);
    if records.is_empty() {
        return;
    }
    let Ok(panes) = tmux::panes() else { return };
    let recorded: HashSet<&str> = records.iter().map(|r| r.pane.as_str()).filter(|p| panes.contains(*p)).collect();
    let shown: HashMap<String, HashSet<u32>> =
        recorded.into_iter().filter_map(|pane| Some((pane.to_owned(), kitty::placeholder_ids(&tmux::capture(pane).ok()??)))).collect();
    let (kept, gone) = sweep(records, &panes, &shown);
    if gone.is_empty() {
        return;
    }
    // The records go only once tmux has them gone: a write that fails leaves them for next time.
    if tmux::set_server_option(OPTION, &format(&kept)).is_ok() {
        for id in gone {
            kitty::delete(out, id, Wrap::Tmux);
        }
    }
}

/// Record `ids`, just printed in this pane, so a later run can delete them.
pub(crate) fn remember(ids: &[u32]) {
    let Some(pane) = tmux::pane() else { return };
    if ids.is_empty() {
        return;
    }
    let Ok(value) = tmux::server_option(OPTION) else { return };
    let mut records = parse(&value);
    records.extend(ids.iter().filter(|id| **id != 0).map(|id| Record { pane: pane.clone(), id: *id }));
    let extra = records.len().saturating_sub(MAX_RECORDED);
    let _ = tmux::set_server_option(OPTION, &format(records.get(extra..).unwrap_or_default()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pane: &str, id: u32) -> Record {
        Record { pane: pane.into(), id }
    }

    #[test]
    fn records_read_back_as_written_and_garbage_is_dropped() {
        let records = vec![record("%3", 402_653_191), record("%12", 7)];
        assert_eq!(parse(&format(&records)), records);
        assert_eq!(parse("%3:1 x %4 :5 %5:0 %6:-1 %7:4294967296 %8:9\n"), [record("%3", 1), record("%8", 9)]);
        // Any name is a pane's, up to MAX_PANE bytes.
        assert_eq!(parse("main:6 %1:2:7 0123456789abcdef:8 0123456789abcdefg:9"), [record("main", 6), record("0123456789abcdef", 8)]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn only_the_newest_records_are_kept() {
        let many: Vec<String> = (1..=MAX_RECORDED + 5).map(|id| format!("%1:{id}")).collect();
        let records = parse(&many.join(" "));
        assert_eq!((records.len(), records.first().map(|r| r.id)), (MAX_RECORDED, Some(6)));
        // The longest record fits tmux's answer that many times over.
        assert!(format(&vec![record(&"%".repeat(MAX_PANE), u32::MAX); MAX_RECORDED]).len() < 64 * 1024);
    }

    /// An image goes when its pane is gone or no longer holds its cells; a pane that couldn't be
    /// read keeps all of its records.
    #[test]
    fn what_no_pane_holds_is_deleted_and_the_rest_stays() {
        let records = vec![record("%1", 10), record("%1", 11), record("%2", 20), record("%3", 30), record("%1", 12)];
        let panes: HashSet<String> = ["%1", "%3"].map(String::from).into();
        // %1 still shows 11 and 12; %3 couldn't be read.
        let shown: HashMap<String, HashSet<u32>> = [("%1".to_string(), [11, 12, 99].into())].into();
        let (kept, gone) = sweep(records, &panes, &shown);
        assert_eq!(kept, [record("%1", 11), record("%3", 30), record("%1", 12)]);
        assert_eq!(gone, [10, 20]);
    }
}
