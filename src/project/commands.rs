//! The effect commands of the pattern's effect and volume columns: their
//! numbers, letters and how they are written.

/// The effect command that picks a phrase, `Z`: base 36 digits write the
/// commands, so this one shows as a letter after the hex ones.
pub const FX_PHRASE: u8 = 35;

/// The effect command that pans the note to and fro, `N`, an auto-pan:
/// speed x, depth y.
pub const FX_AUTOPAN: u8 = 23;

/// The effect command that breaks off the pattern, `J`: the song goes
/// on at line xx of the next slot.
pub const FX_BREAK: u8 = 19;

/// The effect command that sets the track's volume, `L`: 00 silent to
/// 80 full, for every note the track plays until it changes again.
pub const FX_TRACK_VOLUME: u8 = 21;

/// The effect command that plays the sample backwards, `R`: from where
/// it is, or from its end with a note on the line; R00 plays forwards.
pub const FX_REVERSE: u8 = 27;

/// The effect command that plays a slice, `S`: slice xx of the sample
/// the note plays (00 the first), at the note's pitch.
pub const FX_SLICE: u8 = 28;

/// The effect command that stutters the note, `T`, a tremor: on for x
/// ticks, off for y.
pub const FX_TREMOR: u8 = 29;

/// The effect command that holds the song on its line, `W`: for xx
/// lines more, while the line's effects go on.
pub const FX_WAIT: u8 = 32;

/// The effect command letters after the hex digits, by key.
pub fn fx_letter(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        'J' => Some(FX_BREAK),
        'L' => Some(FX_TRACK_VOLUME),
        'N' => Some(FX_AUTOPAN),
        'R' => Some(FX_REVERSE),
        'S' => Some(FX_SLICE),
        'T' => Some(FX_TREMOR),
        'W' => Some(FX_WAIT),
        'Y' => Some(FX_MAYBE),
        'Z' => Some(FX_PHRASE),
        _ => None,
    }
}

/// The volume column's commands, by letter, kept above the volumes (00
/// to 80): sixteen values each from 90 up, the letter's hex digit after it.
pub const VOL_COMMANDS: [char; 7] = ['I', 'O', 'U', 'D', 'G', 'C', 'R'];

/// Where the volume column's commands start.
const VOL_COMMAND_BASE: u8 = 0x90;

/// The volume column command `v` holds, as its letter and digit, if it
/// holds one rather than a volume.
pub fn vol_command(v: u8) -> Option<(char, u8)> {
    let i = v.checked_sub(VOL_COMMAND_BASE)? as usize / 16;
    Some((*VOL_COMMANDS.get(i)?, v & 0xF))
}

/// The volume column value of command `letter` with digit `x`.
pub fn vol_command_value(letter: char, x: u8) -> Option<u8> {
    let i = VOL_COMMANDS.iter().position(|&c| c == letter.to_ascii_uppercase())?;
    Some(VOL_COMMAND_BASE + 16 * i as u8 + (x & 0xF))
}

/// A volume column value as the editor shows it: two hex digits, or a
/// command's letter and digit.
pub fn vol_text(v: u8) -> String {
    match vol_command(v) {
        Some((c, x)) => format!("{c}{x:X}"),
        None => format!("{v:02X}"),
    }
}

/// Reads what `vol_text` writes.
pub fn parse_vol(s: &str) -> Option<u8> {
    let mut chars = s.chars();
    let (a, b) = (chars.next()?, chars.next()?);
    if chars.next().is_some() {
        return None;
    }
    match (vol_command_value(a, 0), b.to_digit(16)) {
        (Some(base), Some(x)) => Some(base + x as u8),
        _ => u8::from_str_radix(s, 16).ok().map(|v| v.min(0x80)),
    }
}

/// The effect command a volume column command stands for: fades are
/// volume slides, pitch slides and glides move x/4 semitone a tick, and
/// cuts and retriggers count x ticks.
pub fn vol_effect(v: u8) -> Option<(u8, u8)> {
    let (c, x) = vol_command(v)?;
    Some(match c {
        'I' => (0xA, x << 4),
        'O' => (0xA, x),
        'U' => (0x1, x * 4),
        'D' => (0x2, x * 4),
        'G' => (0x3, x * 4),
        'C' => (0xC, x),
        _ => (0xE, x),
    })
}

/// The effect command that plays a line's note only sometimes, `Y`: with
/// a chance of xx in FF.
pub const FX_MAYBE: u8 = 34;
