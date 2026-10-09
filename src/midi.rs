//! MIDI keyboard input. Notes arrive on midir's thread and are handed to
//! the UI thread, which plays and records them like keys of the computer
//! keyboard.

use midir::{MidiInput, MidiInputConnection};
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiEvent {
    /// Note and velocity, 1..=127.
    NoteOn(u8, u8),
    NoteOff(u8),
}

/// Reads a note-on or note-off from a MIDI message, on any channel. A
/// note-on with velocity 0 is a note-off, as keyboards send them.
pub fn parse(msg: &[u8]) -> Option<MidiEvent> {
    let [status, note, vel, ..] = *msg else { return None };
    let note = note.min(119);
    match status & 0xF0 {
        0x90 if vel > 0 => Some(MidiEvent::NoteOn(note, vel)),
        0x90 | 0x80 => Some(MidiEvent::NoteOff(note)),
        _ => None,
    }
}

pub struct Midi {
    connection: Option<MidiInputConnection<()>>,
    /// The port connected to.
    pub port: Option<String>,
    tx: Sender<MidiEvent>,
    rx: Receiver<MidiEvent>,
}

impl Default for Midi {
    fn default() -> Self {
        let (tx, rx) = channel();
        Midi { connection: None, port: None, tx, rx }
    }
}

impl Midi {
    /// The MIDI inputs there are, by name; none when MIDI isn't available.
    pub fn ports() -> Vec<String> {
        let Ok(input) = MidiInput::new("noise") else { return Vec::new() };
        input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
    }

    /// Listens to the input called `name`, instead of any other.
    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        self.disconnect();
        let input = MidiInput::new("noise").map_err(|e| e.to_string())?;
        let port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).is_ok_and(|n| n == name))
            .ok_or_else(|| format!("{name} is gone"))?;
        let tx = self.tx.clone();
        let connection = input
            .connect(
                &port,
                "noise input",
                move |_, msg, _| {
                    if let Some(ev) = parse(msg) {
                        let _ = tx.send(ev);
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        self.connection = Some(connection);
        self.port = Some(name.to_string());
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(c) = self.connection.take() {
            c.close();
        }
        self.port = None;
    }

    /// The notes that came in since the last call.
    pub fn poll(&self) -> Vec<MidiEvent> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_on_any_channel() {
        assert_eq!(parse(&[0x90, 60, 100]), Some(MidiEvent::NoteOn(60, 100)));
        assert_eq!(parse(&[0x93, 61, 1]), Some(MidiEvent::NoteOn(61, 1)));
        assert_eq!(parse(&[0x90, 60, 0]), Some(MidiEvent::NoteOff(60)), "velocity 0 lets go");
        assert_eq!(parse(&[0x8F, 60, 64]), Some(MidiEvent::NoteOff(60)));
        assert_eq!(parse(&[0xB0, 7, 100]), None, "controllers are ignored");
        assert_eq!(parse(&[0xF8]), None, "clock is ignored");
        assert_eq!(parse(&[0x90, 127, 100]), Some(MidiEvent::NoteOn(119, 100)), "notes stop at B-9");
    }
}
