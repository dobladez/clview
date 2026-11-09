//! The bookkeeping of a relayout: the document on screen laid out again at a relayout setting's
//! value ([`Effect::Relayout`]), or for the room it is shown in, on the openers' threads, as a new
//! source for a new level. No IO here: which values and room want a layout, when a window that
//! changed size has held still long enough to lay out for, what a job under way comes to when it
//! ends, the pair that failed, and the values to put back then. The pager starts the job, polls it and
//! takes the result in (`Pager::relayout`).

use std::time::{Duration, Instant};

use super::pager::said;
use crate::source::{Effect, Room, Setting, Values};

/// How long a room has to hold still after a change of size before the document is laid out for it:
/// a window being dragged changes size many times a second, and a layout can't be stopped.
pub(super) const HOLD: Duration = Duration::from_millis(500);

/// The relayout settings `has` names, of those `declared`.
fn relayout<'s>(declared: &'s [&'static Setting], has: &'s [&'static str]) -> impl Iterator<Item = &'static Setting> + 's {
    declared.iter().copied().filter(move |setting| setting.effect == Effect::Relayout && has.contains(&setting.name))
}

/// The values of the relayout settings `has`, as `values` have them, at the default where they say
/// nothing: what a layout is compared and asked for by, so that a redraw setting's change tells no two
/// apart.
pub(super) fn of(declared: &[&'static Setting], has: &[&'static str], values: Values) -> Values {
    let mut of = Values::default();
    for setting in relayout(declared, has) {
        // There is room: no more than the build declares.
        let _ = of.set(setting.name, values.get(setting.name).unwrap_or(setting.default));
    }
    of
}

/// What to lay the document out for: the relayout settings `has` as `values` want them, and `room`, the
/// room it is shown in, when either differs from what it is laid out at (`laid`, in `laid_room`; a room
/// is the same within a little, [`Room::same`]). `None` when they agree, or it has no relayout setting.
/// A document laid out for no room is compared by its values alone.
pub(super) fn wanted(
    declared: &[&'static Setting],
    has: &[&'static str],
    values: Values,
    laid: Values,
    room: Room,
    laid_room: Option<Room>,
) -> Option<(Values, Room)> {
    // Nothing to lay out again: no room to speak of either.
    relayout(declared, has).next()?;
    let wanted = of(declared, has, values);
    let moved = laid_room.is_some_and(|laid_room| !laid_room.same(&room));
    (wanted != of(declared, has, laid) || moved).then_some((wanted, room))
}

/// What `values` are said as on the status line, the relayout settings `has` alone, those at their
/// default left out: `width 72`; empty when nothing is said, a layout for a change of size at the
/// defaults.
pub(super) fn describe(declared: &[&'static Setting], has: &[&'static str], values: Values) -> String {
    let each = relayout(declared, has).filter_map(|setting| {
        let value = values.get(setting.name).unwrap_or(setting.default);
        (value != setting.default).then(|| said(setting, value))
    });
    each.reduce(|all, one| format!("{all} · {one}")).unwrap_or_default()
}

/// The room the window has had since it changed, while it isn't the one the document on screen is
/// laid out for: adopted once it has held [`HOLD`] ([`Hold::seen`]).
#[derive(Debug, Default)]
pub(super) struct Hold {
    pending: Option<(Room, Instant)>,
}

impl Hold {
    /// The window's room is `room` at `now`, the document's `current`: the room to lay out for, once
    /// `room` has differed from `current` and held still for [`HOLD`]; `None` meanwhile, and once it is
    /// the same as `current` again (the window dragged back). A room a little off the last one seen
    /// ([`Room::same`]) is the same one, still holding.
    pub(super) fn seen(&mut self, room: Room, current: &Room, now: Instant) -> Option<Room> {
        if room.same(current) {
            self.pending = None;
            return None;
        }
        match self.pending {
            Some((pending, since)) if pending.same(&room) => (now.saturating_duration_since(since) >= HOLD).then(|| {
                self.pending = None;
                room
            }),
            _ => {
                self.pending = Some((room, now));
                None
            }
        }
    }

    /// When the room pending, if one is, will have held long enough: for the pager to wake then.
    pub(super) fn due(&self) -> Option<Instant> {
        self.pending.map(|(_, since)| since + HOLD)
    }
}

/// `values` with the relayout settings `has` back at `laid`'s: after a failure, so that the next step
/// starts from what is on screen.
pub(super) fn put_back(declared: &[&'static Setting], has: &[&'static str], values: Values, laid: Values) -> Values {
    let mut values = values;
    for setting in relayout(declared, has) {
        // There is room: the name is in it already, or there was room for it.
        let _ = values.set(setting.name, laid.get(setting.name).unwrap_or(setting.default));
    }
    values
}

/// The job under way for the level on screen, if one is: the values and the room it lays out for, and
/// `H`, what it is polled by. At most one: a change meanwhile waits in the shared values, and starts
/// when this ends. And the pair that failed for this level, if one did: not asked for again until the
/// values or the room change, or `r`.
pub(super) struct Relayout<H> {
    under_way: Option<(Values, Room, H)>,
    failed: Option<(Values, Room)>,
}

/// What a job's result comes to once it ends ([`Relayout::ended`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ended {
    /// For the values and the room wanted now: taken in, a new level or why not.
    Taken,
    /// For values or a room no longer wanted: dropped, success or failure.
    Stale,
}

impl<H> Relayout<H> {
    pub(super) const fn new() -> Self {
        Self { under_way: None, failed: None }
    }

    /// A job for `values`, for `room`, failed: the pair isn't asked for again ([`Relayout::refused`])
    /// until the values or the room change, or the reader asks again ([`Relayout::retry`]).
    pub(super) fn fail(&mut self, values: Values, room: Room) {
        self.failed = Some((values, room));
    }

    /// Whether `wanted` is the pair that failed, not to be asked for again.
    pub(super) fn refused(&self, wanted: &(Values, Room)) -> bool {
        self.failed.is_some_and(|(values, room)| values == wanted.0 && room.same(&wanted.1))
    }

    /// The reader asks again (`r`): what failed may be tried again.
    pub(super) fn retry(&mut self) {
        self.failed = None;
    }

    /// A job for `values`, for `room`, begun, polled by `handle`.
    pub(super) fn start(&mut self, values: Values, room: Room, handle: H) {
        self.under_way = Some((values, room, handle));
    }

    /// What polls the job under way, if one is.
    pub(super) fn handle(&self) -> Option<&H> {
        self.under_way.as_ref().map(|(_, _, handle)| handle)
    }

    pub(super) fn is_under_way(&self) -> bool {
        self.under_way.is_some()
    }

    /// The job under way ended: whether its result is for `wanted`, the values and the room wanted now
    /// (`None`: what is on screen), or stale. Nothing is under way after.
    pub(super) fn ended(&mut self, wanted: Option<(Values, Room)>) -> Ended {
        match (self.under_way.take(), wanted) {
            (Some((values, room, _)), Some((wanted, wanted_room))) if values == wanted && room.same(&wanted_room) => Ended::Taken,
            _ => Ended::Stale,
        }
    }

    /// The level is left: a job under way runs to its end, and its result is nobody's; what failed was
    /// that level's.
    pub(super) fn forget(&mut self) {
        self.under_way = None;
        self.failed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Keys, Named, Numbers, Value};

    /// A redraw setting and a relayout one, as the documents' backend declares them.
    static PAPER: Setting = crate::source::tests::cycled("paper", 'd', "--theme");
    static WIDTH: Setting = Setting {
        name: "width",
        named: &[Named { name: "window", says: "width window" }],
        numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }),
        default: Value::Named("window"),
        zero: Value::Named("window"),
        auto: None,
        keys: Keys::Step('[', ']'),
        effect: Effect::Relayout,
        ..crate::source::tests::cycled("width", '[', "--width")
    };

    /// The room the tests show the document in, and another.
    const ROOM: Room = Room { cols: 80, rows: 23, cell_w: 10.0, cell_h: 20.0, gap: 2, per_row: None };
    const OTHER: Room = Room { cols: 100, ..ROOM };

    fn values(paper: &'static str, width: Option<u32>) -> Values {
        let mut values = Values::default();
        assert!(values.set("paper", Value::Named(paper)));
        if let Some(width) = width {
            assert!(values.set("width", Value::Number(width)));
        }
        values
    }

    /// A layout is wanted when a relayout setting the document has differs from what it is laid out
    /// at, or the room it is shown in isn't the one it was laid out for, whatever the redraw settings
    /// do; the values asked for are the relayout settings' alone, with the room.
    #[test]
    fn a_layout_is_wanted_when_a_relayout_setting_or_the_room_differs_and_a_redraw_one_doesnt_count() {
        let declared: Vec<&'static Setting> = vec![&PAPER, &WIDTH];
        let has = ["paper", "width"];
        let laid = values("light", None);
        let at = |width| of(&declared, &has, values("dark", Some(width)));
        assert_eq!(wanted(&declared, &has, values("dark", None), laid, ROOM, Some(ROOM)), None, "paper changed: a redraw");
        assert_eq!(wanted(&declared, &has, values("light", Some(84)), laid, ROOM, Some(ROOM)), Some((at(84), ROOM)));
        assert_eq!(at(84).get("paper"), None, "the relayout settings alone");
        assert_eq!(wanted(&declared, &has, values("light", None), values("dark", None), ROOM, Some(ROOM)), None, "both at the default, said or not");
        assert_eq!(wanted(&declared, &["paper"], values("light", Some(84)), laid, ROOM, Some(ROOM)), None, "a document without the setting");
        // The room: another wants one, the same within a little doesn't, and a document laid out for none
        // goes by its values alone.
        let window = of(&declared, &has, laid);
        assert_eq!(wanted(&declared, &has, laid, laid, OTHER, Some(ROOM)), Some((window, OTHER)));
        assert_eq!(wanted(&declared, &has, laid, laid, Room { cell_w: 10.1, ..ROOM }, Some(ROOM)), None);
        assert_eq!(wanted(&declared, &has, laid, laid, OTHER, None), None);
        assert_eq!(wanted(&declared, &["paper"], laid, laid, OTHER, Some(ROOM)), None, "no relayout setting: no room to speak of");
        assert_eq!(describe(&declared, &has, values("light", Some(72))), "width 72");
        assert_eq!(describe(&declared, &has, values("light", None)), "", "the default is left unsaid");
        assert_eq!(describe(&declared, &has, values("light", Some(72))), describe(&declared, &has, values("dark", Some(72))));
        assert_eq!(put_back(&declared, &has, values("dark", Some(72)), values("light", Some(40))), values("dark", Some(40)));
        assert_eq!(put_back(&declared, &has, values("dark", Some(72)), values("light", None)).get("width"), Some(Value::Named("window")));
    }

    /// One job under way at a time: a change meanwhile, of a value or of the room, makes its result
    /// stale when it ends, dropped whether it succeeded or failed, and the next starts then; one for the
    /// values and the room still wanted is taken in; leaving the level forgets it.
    #[test]
    fn a_result_is_taken_for_the_values_and_room_still_wanted_and_dropped_otherwise() {
        let declared: Vec<&'static Setting> = vec![&PAPER, &WIDTH];
        let has = ["paper", "width"];
        let at = |width| of(&declared, &has, values("light", Some(width)));
        let mut relayout: Relayout<u8> = Relayout::new();
        assert!(!relayout.is_under_way() && relayout.handle().is_none());
        relayout.start(at(84), ROOM, 1);
        assert_eq!((relayout.is_under_way(), relayout.handle()), (true, Some(&1)));
        assert_eq!(relayout.ended(Some((at(84), ROOM))), Ended::Taken);
        assert!(!relayout.is_under_way());
        // Changed meanwhile, to 88, or back to what is on screen: stale either way.
        relayout.start(at(84), ROOM, 2);
        assert_eq!(relayout.ended(Some((at(88), ROOM))), Ended::Stale);
        relayout.start(at(84), ROOM, 3);
        assert_eq!(relayout.ended(None), Ended::Stale);
        // The room changed meanwhile: stale; by a little: still the one wanted.
        relayout.start(at(84), ROOM, 4);
        assert_eq!(relayout.ended(Some((at(84), OTHER))), Ended::Stale);
        relayout.start(at(84), ROOM, 5);
        assert_eq!(relayout.ended(Some((at(84), Room { cell_h: 20.2, ..ROOM }))), Ended::Taken);
        // The paper changed meanwhile: still the values wanted.
        relayout.start(at(84), ROOM, 6);
        assert_eq!(relayout.ended(Some((of(&declared, &has, values("dark", Some(84))), ROOM))), Ended::Taken);
        relayout.start(at(84), ROOM, 7);
        relayout.forget();
        assert!(!relayout.is_under_way());
        assert_eq!(relayout.ended(Some((at(84), ROOM))), Ended::Stale, "nothing under way");
    }

    /// A pair that failed isn't asked for again until the values or the room change, or `r`; leaving
    /// the level forgets it too, as what failed was that level's.
    #[test]
    fn a_pair_that_failed_is_not_asked_for_again_until_either_changes_or_r() {
        let declared: Vec<&'static Setting> = vec![&PAPER, &WIDTH];
        let has = ["paper", "width"];
        let at = |width| of(&declared, &has, values("light", Some(width)));
        let mut relayout: Relayout<u8> = Relayout::new();
        assert!(!relayout.refused(&(at(84), ROOM)));
        relayout.fail(at(84), ROOM);
        assert!(relayout.refused(&(at(84), ROOM)) && relayout.refused(&(at(84), Room { cell_h: 20.3, ..ROOM })), "the same, within a little");
        assert!(!relayout.refused(&(at(88), ROOM)) && !relayout.refused(&(at(84), OTHER)), "either changed");
        relayout.retry();
        assert!(!relayout.refused(&(at(84), ROOM)));
        relayout.fail(at(84), ROOM);
        relayout.forget();
        assert!(!relayout.refused(&(at(84), ROOM)));
    }

    /// A room that differs from the document's is adopted once it has held still for `HOLD`: a window
    /// still being dragged starts the wait again, one dragged back to the document's room ends it, and a
    /// cell a little off is the same room, still holding.
    #[test]
    fn a_changed_room_is_laid_out_for_once_it_has_held_still() {
        let start = Instant::now();
        let at = |millis: u64| start + Duration::from_millis(millis);
        let mut hold = Hold::default();
        assert_eq!((hold.seen(ROOM, &ROOM, at(0)), hold.due()), (None, None), "the document's own");
        assert_eq!(hold.seen(OTHER, &ROOM, at(0)), None);
        assert_eq!(hold.due(), Some(at(500)));
        assert_eq!(hold.seen(OTHER, &ROOM, at(400)), None, "not yet");
        assert_eq!(hold.seen(Room { cell_w: 10.1, ..OTHER }, &ROOM, at(450)), None, "the same room, a little off");
        assert_eq!(hold.seen(OTHER, &ROOM, at(500)), Some(OTHER));
        assert_eq!(hold.due(), None, "adopted: nothing pending");
        // Dragged on: the wait starts again from the newest room.
        let wider = Room { cols: 120, ..ROOM };
        assert_eq!(hold.seen(OTHER, &ROOM, at(1000)), None);
        assert_eq!(hold.seen(wider, &ROOM, at(1300)), None);
        assert_eq!(hold.seen(wider, &ROOM, at(1600)), None, "300 ms of the new one");
        assert_eq!(hold.seen(wider, &ROOM, at(1800)), Some(wider));
        // Dragged back to the document's room: nothing to do.
        assert_eq!(hold.seen(OTHER, &ROOM, at(2000)), None);
        assert_eq!((hold.seen(ROOM, &ROOM, at(2100)), hold.due()), (None, None));
        assert_eq!(hold.seen(OTHER, &ROOM, at(2200)), None, "and the wait starts afresh");
        assert_eq!(hold.seen(OTHER, &ROOM, at(2600)), None);
        assert_eq!(hold.seen(OTHER, &ROOM, at(2700)), Some(OTHER));
    }
}
