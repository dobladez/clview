//! The bookkeeping of a relayout: the document on screen laid out again at a relayout setting's
//! value ([`Effect::Relayout`]), on the openers' threads, as a new source for a new level. No IO
//! here: which values want a layout, what a job under way comes to when it ends, and the values to put
//! back when one fails. The pager starts the job, polls it and takes the result in (`Pager::relayout`).

use super::pager::said;
use crate::source::{Effect, Room, Setting, Values};

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

/// What `values` are said as on the status line, the relayout settings `has` alone: `width 72`.
pub(super) fn describe(declared: &[&'static Setting], has: &[&'static str], values: Values) -> String {
    let each = relayout(declared, has).map(|setting| said(setting, values.get(setting.name).unwrap_or(setting.default)));
    each.reduce(|all, one| format!("{all} · {one}")).unwrap_or_default()
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
/// when this ends.
pub(super) struct Relayout<H> {
    under_way: Option<(Values, Room, H)>,
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
        Self { under_way: None }
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

    /// The level is left: a job under way runs to its end, and its result is nobody's.
    pub(super) fn forget(&mut self) {
        self.under_way = None;
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
        numbers: Some(Numbers { least: 20, most: 200, step: 4 }),
        default: Value::Named("window"),
        zero: Value::Named("window"),
        auto: None,
        keys: Keys::Step('[', ']'),
        effect: Effect::Relayout,
        ..crate::source::tests::cycled("width", '[', "--width")
    };

    /// The room the tests show the document in, and another.
    const ROOM: Room = Room { cols: 80, rows: 23, cell_w: 10.0, cell_h: 20.0, gap: 2 };
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
        assert_eq!(describe(&declared, &has, values("light", None)), "width window");
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
}
