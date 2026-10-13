//! The song's key and scale, which notes typed snap to, and the chords one
//! key can write.

use serde::{Deserialize, Serialize};

/// The scales, by name, as the semitones above the key note they hold.
pub const SCALES: &[(&str, &[u8])] = &[
    ("Chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ("Major", &[0, 2, 4, 5, 7, 9, 11]),
    ("Minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("Harmonic Minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("Melodic Minor", &[0, 2, 3, 5, 7, 9, 11]),
    ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("Phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("Lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("Locrian", &[0, 1, 3, 5, 6, 8, 10]),
    ("Major Pentatonic", &[0, 2, 4, 7, 9]),
    ("Minor Pentatonic", &[0, 3, 5, 7, 10]),
    ("Blues", &[0, 3, 5, 6, 7, 10]),
];

/// The names of the twelve notes, for the key.
pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// The highest note a cell holds.
const TOP_NOTE: u8 = 119;

/// The song's key and scale. Chromatic, the default, holds every note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scale {
    /// The key note, 0 for C up to 11 for B.
    pub key: u8,
    /// Which of `SCALES`.
    pub kind: usize,
}

impl Scale {
    pub fn is_chromatic(&self) -> bool {
        self.kind == 0
    }

    pub fn name(&self) -> String {
        if self.is_chromatic() {
            return SCALES[0].0.into();
        }
        format!("{} {}", NOTE_NAMES[self.key as usize % 12], SCALES[self.kind.min(SCALES.len() - 1)].0)
    }

    fn steps(&self) -> &'static [u8] {
        SCALES[self.kind.min(SCALES.len() - 1)].1
    }

    pub fn contains(&self, note: u8) -> bool {
        self.steps().contains(&((note + 12 - self.key % 12) % 12))
    }

    /// The note of the scale nearest `note`, the lower one when two are as
    /// near.
    pub fn snap(&self, note: u8) -> u8 {
        (0..12u8)
            .flat_map(|d| [note.checked_sub(d), note.checked_add(d).filter(|&n| n <= TOP_NOTE)])
            .flatten()
            .find(|&n| self.contains(n))
            .unwrap_or(note)
    }

    /// The note `degrees` steps of the scale above `note`, which is in it.
    fn up(&self, note: u8, degrees: usize) -> u8 {
        let mut n = note;
        for _ in 0..degrees {
            n = (n + 1..=TOP_NOTE).find(|&m| self.contains(m)).unwrap_or(TOP_NOTE);
        }
        n
    }
}

/// What a chord stacks on the note played: semitones above it, or steps
/// of the song's scale above it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chord {
    Semitones(&'static [u8]),
    InKey(&'static [usize]),
}

/// The chords a key can write, by name; the first is a single note.
pub const CHORDS: &[(&str, Chord)] = &[
    ("Off", Chord::Semitones(&[])),
    ("Triad in Key", Chord::InKey(&[2, 4])),
    ("Seventh in Key", Chord::InKey(&[2, 4, 6])),
    ("Major", Chord::Semitones(&[4, 7])),
    ("Minor", Chord::Semitones(&[3, 7])),
    ("Diminished", Chord::Semitones(&[3, 6])),
    ("Augmented", Chord::Semitones(&[4, 8])),
    ("Sus2", Chord::Semitones(&[2, 7])),
    ("Sus4", Chord::Semitones(&[5, 7])),
    ("Major 7", Chord::Semitones(&[4, 7, 11])),
    ("Minor 7", Chord::Semitones(&[3, 7, 10])),
    ("Dominant 7", Chord::Semitones(&[4, 7, 10])),
    ("Power", Chord::Semitones(&[7, 12])),
    ("Octave", Chord::Semitones(&[12])),
];

/// The notes chord `chord` (of `CHORDS`) plays on `note`, which comes
/// first, in `scale`; notes above the top are left out.
pub fn chord_notes(chord: usize, note: u8, scale: Scale) -> Vec<u8> {
    let mut notes = vec![note];
    match CHORDS.get(chord).map_or(Chord::Semitones(&[]), |c| c.1) {
        Chord::Semitones(steps) => notes.extend(steps.iter().map(|&s| note.saturating_add(s))),
        Chord::InKey(degrees) => {
            let root = scale.snap(note);
            notes.extend(degrees.iter().map(|&d| scale.up(root, d)));
        }
    }
    let mut seen = Vec::new();
    notes.retain(|&n| {
        n <= TOP_NOTE && !seen.contains(&n) && {
            seen.push(n);
            true
        }
    });
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    const C_MAJOR: Scale = Scale { key: 0, kind: 1 };

    #[test]
    fn notes_snap_to_the_nearest_in_the_scale() {
        assert_eq!(C_MAJOR.snap(60), 60, "C stays");
        assert_eq!(C_MAJOR.snap(61), 60, "C# goes down to C, as near as D");
        assert_eq!(C_MAJOR.snap(66), 65, "F# to F");
        let a_minor_pentatonic = Scale { key: 9, kind: 11 };
        assert_eq!(a_minor_pentatonic.snap(59), 60, "B to C, the nearer");
        assert_eq!(Scale::default().snap(61), 61, "chromatic holds every note");
        assert_eq!(Scale { key: 2, kind: 1 }.name(), "D Major");
        assert_eq!(
            (super::super::Note::On(60).label(), super::super::Note::On(61).label()),
            ("C-5".into(), "C#5".into())
        );
    }

    #[test]
    fn chords_stack_semitones_or_steps_of_the_scale() {
        let major = CHORDS.iter().position(|c| c.0 == "Major").unwrap();
        assert_eq!(chord_notes(major, 62, C_MAJOR), [62, 66, 69], "D major, whatever the key");
        let triad = CHORDS.iter().position(|c| c.0 == "Triad in Key").unwrap();
        assert_eq!(chord_notes(triad, 62, C_MAJOR), [62, 65, 69], "D minor in C major");
        assert_eq!(chord_notes(triad, 71, C_MAJOR), [71, 74, 77], "B diminished");
        let seventh = CHORDS.iter().position(|c| c.0 == "Seventh in Key").unwrap();
        assert_eq!(chord_notes(seventh, 67, C_MAJOR), [67, 71, 74, 77], "G7");
        assert_eq!(chord_notes(0, 60, C_MAJOR), [60], "off: the note alone");
        assert_eq!(chord_notes(major, 118, C_MAJOR), [118], "nothing past the top");
    }
}
