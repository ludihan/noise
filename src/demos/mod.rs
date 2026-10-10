//! The demo songs, each built here in the program's own terms, and what
//! they share: notes and effects in cells, envelopes, device chains,
//! rolls and held notes, and the sounds in `sounds`.

mod clockwork_rain;
mod concrete_hymn;
mod last_light;
mod prism_overdrive;
mod sounds;
mod static_heart;

use crate::project::*;
use crate::sample::Sample;
use sounds::*;

/// A slot holding audio rendered here, to be written out when saved.
fn rendered(sample: Sample) -> SampleSlot {
    SampleSlot { unsaved: true, ..SampleSlot::new(sample, None) }
}

fn n(note: u8, module: u8, vol: u8) -> Cell {
    Cell { note: Some(Note::On(note)), module: Some(module), vol: Some(vol), ..Cell::default() }
}

fn fx(cell: Cell, cmd: u8, arg: u8) -> Cell {
    Cell { fx: Some((cmd, arg)), ..cell }
}

fn off() -> Cell {
    Cell { note: Some(Note::Off), ..Cell::default() }
}

/// An envelope for parameter `param` of module `m` of kind `kind`, from
/// (line, value) points in the parameter's own units.
fn envelope(m: u8, kind: ModuleKind, param: usize, points: &[(f32, f32)], steps: bool, curve: bool) -> Envelope {
    let spec = kind.automatable(param).expect("an automatable parameter");
    let points = points.iter().map(|&(l, v)| (l, spec.position(v))).collect();
    Envelope { steps, curve, ..Envelope::new(m, param, points) }
}

// ---------------------------------------------------------------- shared by the demo songs

/// Names module `id` and sets its parameters, as (index, value).
fn set(p: &mut Project, id: u8, name: &str, params: &[(usize, f32)]) {
    let m = p.module_mut(id).unwrap();
    m.name = name.into();
    for &(i, v) in params {
        m.params[i] = v;
    }
}

/// Adds a `kind` module called `name`, with `params` set.
fn add(p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]) -> u8 {
    let id = p.add_module(kind, [0.0, 0.0]).unwrap();
    set(p, id, name, params);
    id
}

/// Sends the end of `id`'s chain to `to` rather than the output.
fn send(p: &mut Project, id: u8, to: u8) {
    let last = p.chain(id).effects.last().copied().unwrap_or(id);
    p.disconnect(last, OUTPUT_ID);
    p.connect(last, to);
}

/// An effect to add: its kind, name and parameters, as `set` takes them.
pub(crate) type Effect<'a> = (ModuleKind, &'a str, &'a [(usize, f32)]);

/// Adds `list` to the end of `owner`'s chain; returns the effects' ids.
fn effects<const N: usize>(p: &mut Project, owner: impl Into<Owner>, list: [Effect; N]) -> [u8; N] {
    let owner = owner.into();
    let start = p.chain(owner).effects.len();
    std::array::from_fn(|k| {
        let (kind, name, params) = list[k];
        let id = p.chain_insert(owner, start + k, kind).unwrap();
        set(p, id, name, params);
        id
    })
}

/// A roll: `cell`'s note on each of `lines` lines from `line`, played again
/// every `rate` ticks (Exx) in between, rising from its volume to `to`.
fn roll(pat: &mut Pattern, track: usize, line: usize, lines: usize, cell: Cell, rate: u8, to: u8) {
    let from = cell.vol.unwrap_or(0x40);
    for k in 0..lines {
        let vol = from + (to.saturating_sub(from) as usize * k / lines) as u8;
        pat.tracks[track][line + k] = fx(Cell { vol: Some(vol), ..cell }, 0xE, rate);
    }
}

/// The Repeater `id` holds during each of `holds` (from, to), its loop as
/// long as `lengths` say: (line, index into `REPEATER_LENGTHS`).
fn hold(pat: &mut Pattern, id: u8, holds: &[(f32, f32)], lengths: &[(f32, f32)]) {
    let r = ModuleKind::Repeater;
    let mut points = Vec::new();
    if holds.first().is_none_or(|h| h.0 > 0.0) {
        points.push((0.0, 0.0));
    }
    for &(from, to) in holds {
        points.extend([(from, 1.0), (to, 0.0)]);
    }
    pat.automation.push(envelope(id, r, 0, &points, true, false));
    pat.automation.push(envelope(id, r, 1, lengths, true, false));
}

/// `notes` (line, note) on column `col` of `track`.
fn tune(pat: &mut Pattern, track: usize, col: usize, id: u8, notes: &[(usize, u8)], vol: u8) {
    for &(l, note) in notes {
        *pat.cell_mut(track, col, l) = n(note, id, vol);
    }
}

/// Lets go of the notes still sounding on the note columns of `tracks`,
/// as (track, columns), at the pattern's start.
fn let_go(pat: &mut Pattern, tracks: &[(usize, usize)]) {
    for &(t, cols) in tracks {
        for c in 0..cols {
            *pat.cell_mut(t, c, 0) = off();
        }
    }
}
