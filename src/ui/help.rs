//! The manual: how every part of the program works, the keys and the
//! effect commands, by topic, with a search across them. Opened with F1.

use super::theme;
use eframe::egui::{self, RichText};

/// What a topic is made of.
enum Block {
    /// A paragraph.
    P(&'static str),
    /// A heading inside a topic.
    H(&'static str),
    /// Bullet points.
    List(&'static [&'static str]),
    /// Two columns: a key, command or name, and what it does.
    Table(&'static [(&'static str, &'static str)]),
    /// Monospaced lines, as for the parts of a pattern cell.
    Code(&'static str),
}

use Block::{Code, H, List, P, Table};

struct Topic {
    title: &'static str,
    blocks: &'static [Block],
}

const KEYS: &[(&str, &str)] = &[
    ("Space", "Play / stop"),
    ("Shift+Space", "Play from the cursor's line"),
    ("Esc", "Toggle edit mode"),
    ("Z–M, Q–P", "Play notes (a two-row piano layout)"),
    ("A", "Note off"),
    ("- / =", "Octave down / up"),
    ("Arrows, Tab, Shift+Tab", "Move the cursor"),
    ("PgUp / PgDn, Home / End", "Jump through the pattern"),
    ("Del", "Clear the field under the cursor, or the selected block"),
    ("Backspace", "Clear the line above and step onto it"),
    ("Ins / Shift+Backspace", "Push rows down / pull rows up"),
    ("0–9, A–F", "Hex values in the module, volume and effect columns"),
    ("J, L, N, R, S, T, W, Y, Z", "The commands written with letters, in the effect column"),
    ("I, O, U, D, G, C, R", "The volume column's commands, in its first digit"),
    ("Shift+arrows, drag", "Select a block; right-click it for more"),
    ("Ctrl+A", "Select the track, then the whole pattern"),
    ("Ctrl+C / Ctrl+X / Ctrl+V", "Copy / cut / paste the block"),
    ("Ctrl+Shift+V", "Mix paste: fill only empty fields"),
    ("Ctrl+F1 / Ctrl+F2", "Transpose the block a semitone down / up"),
    ("Ctrl+F11 / Ctrl+F12", "Transpose the block an octave down / up"),
    ("Ctrl+I", "Interpolate volumes and effects"),
    ("Alt+Left / Alt+Right", "Previous / next note column"),
    ("Alt+L", "Block loop on / off"),
    ("Alt+Up / Alt+Down", "Move the block loop by its length"),
    ("Ctrl+click a matrix block", "Mute the track in that slot of the song"),
    ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
    ("Ctrl+N", "New song"),
    ("Ctrl+O", "Open a song"),
    ("Ctrl+S / Ctrl+Shift+S", "Save / save as"),
    ("Ctrl+Q", "Quit"),
    ("Ctrl+Up / Ctrl+Down", "Previous / next instrument"),
    ("Ctrl+1 … Ctrl+5", "Show or hide the upper frame, sequencer, lower frame, instruments, disk browser"),
    ("F1", "This manual"),
    ("F2 / F3 / F4", "Pattern editor / mixer / instrument editor"),
];

const EFFECTS: &[(&str, &str)] = &[
    ("0xy", "Arpeggio: cycle through the note, +x and +y semitones"),
    ("1xx / 2xx", "Slide pitch up / down by xx/16 semitone per tick"),
    ("3xx", "Glide to the note at xx/16 semitone per tick"),
    ("4xy", "Vibrato: speed x, depth y/8 semitone (0 keeps the last value)"),
    ("7xy", "Tremolo: speed x, depth y/16 of the volume (0 keeps the last value)"),
    ("8xx", "Pan the track's notes: 00 left, 80 middle, FF right"),
    ("9xx", "Sampler: start playback xx/256 of the way into the sample"),
    ("Axy", "Slide the volume up by x, or down by y, volume steps per tick"),
    ("Bxx", "Jump to position xx of the song after this line"),
    ("Cxx", "Cut the note after xx ticks"),
    ("Dxx", "Delay the note by xx ticks"),
    ("Exx", "Play the note again every xx ticks"),
    ("Fxx", "Set the BPM to xx (20 or more), or the ticks per line (below 20); F00 ends the song after this line"),
    (
        "Jxx",
        "Break off the pattern: the song goes on at line xx (hex: J10 is line 16) of the next slot; with Bxx, of that slot",
    ),
    (
        "Lxx",
        "Track volume: 00 silent to 80 full, for every note the track plays, sounding or to come, until changed; it starts full each time the song plays",
    ),
    ("Nxy", "Auto-pan: swing the note's panning at speed x, depth y/F (0 keeps the last value)"),
    (
        "Rxx",
        "Sampler: play the note backwards, from the sample's end with a note on the line, or from where it is; R00 plays forwards again",
    ),
    (
        "Sxx",
        "Sampler: play slice xx (00 the first) of the note's sample at the note's pitch; a note that plays no sample plays it from the first sliced sample, pitched from its base note",
    ),
    ("Txy", "Tremor: the note sounds for x ticks, then is silent for y, over and over through the line"),
    ("Wxx", "Wait: hold the song on this line for xx lines more, while the line's slides and other effects go on"),
    ("Yxx", "Maybe play the note: with a chance of xx in FF (FF always, 00 never)"),
    ("Zxx", "Play the note with the instrument's phrase xx (from 01), or Z00 without one"),
];

/// The volume column's commands, as `project::vol_effect` plays them.
const VOL_COMMANDS: &[(&str, &str)] = &[
    ("Ix", "Fade in: the volume slides up by x volume steps per tick (as Ax0)"),
    ("Ox", "Fade out: the volume slides down by x steps per tick (as A0x); with a note it starts from full"),
    ("Ux / Dx", "Slide the pitch up / down by x/4 semitone per tick"),
    ("Gx", "Glide to the note at x/4 semitone per tick (as 3xx)"),
    ("Cx", "Cut the note after x ticks (as Cxx)"),
    ("Rx", "Play the note again every x ticks (as Exx)"),
];

const MODULES: &[(&str, &str)] = &[
    ("Generator", "Saw, square, triangle, sine or noise, with an ADSR envelope and unison"),
    (
        "Analog Synth",
        "A subtractive synth: two oscillators (saw, square, triangle or sine; the second tuned and detuned), a sub an octave down and noise, into a ladder filter on every voice with its own envelope, key tracking and velocity; an amp envelope, an LFO on the pitch, cutoff and pulse width, the filter envelope on the pitch, unison, and Poly, Mono or Legato voices with glide",
    ),
    (
        "Plucked String",
        "A plucked string: a burst of noise goes round a delay one cycle of the note long, losing its highs as it rings. Position is where it is plucked, Brightness how sharp the pluck is (softer notes pluck darker), Decay how long it rings, Damping how fast its highs go; up to three strings a voice, Detune apart",
    ),
    (
        "Wavetable",
        "Waves from a table (Basic, Pulse, Sync, Fold or Vocal) that Position morphs through and Sweep moves while a note sounds, band-limited for every note, with up to seven detuned unison voices spread in stereo, an ADSR envelope and Modulation",
    ),
    ("FM", "Two-operator FM synth with a decaying modulator"),
    ("Drums", "C kick, D snare, F# closed hi-hat, A# open hi-hat, other notes a tom"),
    ("Kicker", "A kick drum: a wave falling from octaves above the note to it, with a boost"),
    (
        "SpectraVoice",
        "Additive synthesis from up to 32 harmonics, with their slope, the even ones' level, stretch and shimmer",
    ),
    ("FMX", "Four-operator FM: eight algorithms, each operator with its ratio, level and envelope, and feedback"),
    ("Sampler", "Samples mapped across the keyboard, with loops and slices, or a soundfont (see Sampler)"),
    (
        "Granular",
        "Plays its sample as a cloud of short grains around a point Scan moves through it (0 holds still), as pads and textures: grain Size and Density, Spray, random pitch, stereo spread and reversed grains",
    ),
    (
        "Input",
        "Plays the sound card's input, live: a microphone or an instrument through the effects. The input is open while the song has an Input module",
    ),
    ("MultiSynth", "Passes the notes it gets on to the instruments it is connected to"),
    (
        "Glide",
        "Passes notes on to the instruments it is connected to, sliding each from the last note on its column in Time, whatever the distance; Legato slides only a note played over the last one, without starting it again",
    ),
    ("Modulator", "Moves a parameter of each module it links to, with an LFO or by following its input"),
    ("Filter", "Lowpass, highpass or bandpass with resonance and an LFO"),
    ("Distortion", "Soft clip, hard clip or wave fold with a tone control, bit crushing and downsampling"),
    ("Delay", "Ping-pong delay, timed in pattern lines"),
    ("Reverb", "Freeverb-style reverb"),
    ("Amplifier", "Volume, pan and phase invert"),
    ("LFO", "Tremolo or auto-pan with five shapes, in Hz or synced to a period in lines"),
    ("Flanger", "Flanger with feedback, or a stereo chorus"),
    ("Phaser", "Up to 12 allpass stages swept between a floor and a ceiling, with feedback"),
    ("Vocal Filter", "Formant filters that make a sound say A, E, I, O or U, morphing between them"),
    (
        "Vocoder",
        "Splits its Key (a voice, drums) into bands and gives each band of its own input (a synth) the key's level there, so the synth speaks with the key's voice: bands, the range they cover, how sharp they are, how fast they follow, noise for consonants, gain and mix",
    ),
    ("Repeater", "While Hold is on, loops the last 8 lines to 1/16 line of its input: stutters"),
    ("Ring Mod", "The sound times a carrier wave, for bell and robot tones"),
    (
        "Gate",
        "Silences the sound while it is quieter than a threshold, with attack, hold, release and a floor. Its Key can be another module: then that sound opens and closes it (a sidechain)",
    ),
    ("Pitch Shifter", "Moves the sound up or down by up to two octaves, with a grain size and feedback"),
    ("Stereo Expander", "Makes the sound wider or narrower, keeping the lows below Mono bass in the middle"),
    ("Comb Filter", "Rings at a note and its harmonics, with feedback and damping"),
    ("Maximizer", "Boosts the sound and holds its peaks under a ceiling, looking ahead"),
    ("Exciter", "Drives the highs above a frequency and adds them back, for air and presence"),
    ("DC Blocker", "Takes away an offset below hearing, which distortion and ring modulation can leave"),
    ("Scream Filter", "A resonant filter driven hard, with the distortion inside its loop, for growls and screams"),
    ("Multitap Delay", "Four echoes, each with its own time in lines, level and pan, with feedback"),
    ("EQ 10", "A graphic EQ: ten bands an octave apart, from 31 Hz to 16 kHz"),
    (
        "Cabinet Simulator",
        "A little drive and the tone of a speaker cabinet: a small combo, a 4x12, a bass 1x15 or a radio",
    ),
    ("Vibrato", "Bends the pitch up and down with a swinging delay, with a stereo offset"),
    ("WaveShaper", "Bends the sound through a curve of nine points, mirrored or not, with input and output levels"),
    ("Filter Pro", "Eight filter types, 12 to 48 dB an octave, with resonance, and gain for the peak and shelves"),
    ("Chorus", "Up to four voices sweeping around each other, with a stereo offset and feedback"),
    ("Echo", "A delay timed in seconds, darker each time round, the right side later"),
    (
        "Analog Filter",
        "A Moog-style ladder with drive and resonance that sings at full: lowpass 24 or 12 dB, bandpass, highpass",
    ),
    (
        "Convolver",
        "The sound through an impulse response: a made-up room, hall, plate, spring or speaker cabinet, or any sample loaded as one (Load Impulse… under its settings), with no delay",
    ),
    ("Plate Reverb", "A dense plate reverb after Dattorro: predelay, decay, damping and width"),
    ("EQ 5", "A parametric EQ: a low shelf, three peaks and a high shelf, each with its frequency, gain and width"),
    (
        "Compressor",
        "Threshold, ratio, attack, release, makeup gain and mix. Its Key can be another module, whose sound then turns it down (a sidechain): a kick ducking a pad or bass",
    ),
    ("EQ", "A low shelf, a mid peak and a high shelf, each with its frequency"),
    ("Output", "The final mix"),
];

const TOPICS: &[Topic] = &[
    Topic {
        title: "Getting started",
        blocks: &[
            P(
                "noise is a music tracker with modular synths: a pattern editor and an instrument editor over synth and effect modules. Notes in the pattern go straight to synth modules, and each instrument's sound goes through a chain of effect modules.",
            ),
            P(
                "It opens with a demo song, Concrete Hymn: overdriven electro, a clipped square bass riff and crushed drums pumping under the kick, wide pulse stabs, dead stops, and an organ hymn with a gliding siren over it. Press Space to hear it; View > Song Comments says what to look at in it. File > Demo Songs opens it again, or Last Light: two minutes that use nearly everything noise has; or Static Heart: glitchy, bitcrushed hyperpop with a fuzz wall, a gliding pitched-up voice, sliced vocal chops and breakbeat, gated and ducked by Modulators, ending in a tape stop; or Clockwork Rain: IDM at eight lines a beat, a music box over a breakbeat sliced at every hit and chopped in 32nds, with rolls, slice walks and phrases in triplets and fives, acid, a fretless bass, a part in 7/8, and a storm that speeds up until the master freezes; or Prism Overdrive: rhythm-game hardcore with a supersaw hook pumping under a hardcore kick, a kit with a sample on keys of its own for each sound, a psytrance part, a half-time growl, a bar at 255 BPM, and a last chorus a whole tone up.",
            ),
            H("The window"),
            List(&[
                "Menu bar: File, Edit (undo, redo, block operations, MIDI input), View (views, parts of the window, Song Comments) and Help.",
                "Transport: play (the pattern from its top), play from the cursor's line, loop pattern, stop and edit mode; Follow, Metronome and Panic; the song settings; the note entry settings; the song position and the CPU load.",
                "Upper frame: the master scope with peak meters, the track scopes, the spectrum or the wave view.",
                "Left: the pattern sequencer, and in its extended view the pattern matrix.",
                "Middle: the pattern editor (F2), the mixer (F3) or the instrument editor (F4).",
                "Lower frame: the module list with the selected module's parameters, the automation editor, or the track and master effects, opened with the tabs along the bottom.",
                "Right: the instrument list, with the disk browser below it. Drag the column's edge to widen it.",
            ]),
            P(
                "Notes play on the instrument selected in the instrument list; in edit mode (Esc) they are written to the pattern as well.",
            ),
            P(
                "Recording live: in edit mode with the song playing and Follow on, notes go to the line playing. Keys held together go to the track's next free note columns, adding one when all are taken, as a chord; releasing a key writes a note-off (Edit > Record Note-Offs). Edit > Record Quantize puts them on the nearest multiple of 1 to 16 lines; off, a track that shows its delay column gets how late in the line each was played.",
            ),
        ],
    },
    Topic {
        title: "Window and files",
        blocks: &[
            H("Showing and hiding parts"),
            P(
                "Parts of the window can be hidden where they are: the arrows at the ends of the Pattern Editor, Mixer and Instrument tabs show and hide the sequencer on the left, the scopes above and the instrument list and disk browser on the right. The lower frame's tabs stay along the bottom, and the frame starts closed: a tab opens it on its page, at the height it had, and clicking the open one closes it. The View menu and Ctrl+1 to Ctrl+5 (upper frame, sequencer, lower frame, instrument list, disk browser) do the same, and View > Transport hides groups of the transport.",
            ),
            H("Transport"),
            List(&[
                "BPM, LPB (lines per beat) and TPL (ticks per line: the steps effects take within a line).",
                "SWING, the groove: every odd line starts up to half a line late, while the even lines stay on time. Double-click it for none.",
                "The song starts again after its last slot. An F00 effect ends it instead: playing stops after that line and lets the last notes ring out, and the next play starts from the top; the demo song ends that way. Renders stop at the end of the song either way.",
                "OCT, STEP (lines the cursor moves after a note) and VOL, the volume written with the notes you enter (-- writes none).",
                "Metronome clicks every beat while playing; Panic silences everything.",
                "MASTER, on the right, is the volume of everything: drag it (Shift for fine steps), double-click it for its default.",
            ]),
            H("Scopes"),
            P(
                "Scope shows the master output with peak meters; Tracks shows a scope per track in its color, of what the track's notes play before effects (click one to mute the track, right-click to solo it); Spectrum shows the output from 20 Hz to 20 kHz; Wave gives every sample sounding a lane with its waveform, loop and a playhead per voice.",
            ),
            H("Instrument list"),
            P(
                "Every module that plays notes, by number. + adds an instrument connected to the output, Dup copies the selected one with its connections, − deletes it. M and S mute and solo. Double-click an instrument to rename it; right-click for more.",
            ),
            H("Disk browser"),
            P(
                "Three categories: Songs, Instruments (SF2, SF3 and SFZ soundfonts) and Samples (WAV, FLAC, Ogg Vorbis), each remembering its own folder. Below them, the folder's path with buttons for the folder above, home and the music folder; then its folders, a click going into one, apart from its files, each with its type and size. Click a sample to hear it; double-click a sample or soundfont to load it into the selected Sampler, or a song to open it. It starts in your music folder (or home), or the folder of the song you opened, and remembers where you leave each category.",
            ),
            P(
                "Under the samples, a playback strip shows what the preview plays and how far it has got. Play plays the selected sample and Stop stops it; Auto plays samples as soon as they are clicked, in the file explorer and in the Sampler's sample list too (a slice plays on its own); Loop plays previews over and over until stopped; the slider sets how loud previews are. These are kept between sessions.",
            ),
            H("Files"),
            P(
                "Open, Save As, Import Project, Export Project, Render to WAV and the Sampler's Load… use the built-in file explorer: browse folders or jump to Home, the song's folder or the working folder, click a file or type a name. Clicking an audio file while loading samples plays it. The window title marks unsaved changes with *, and New, Open and Quit ask whether to save them. While a song has unsaved changes it is backed up every three minutes, in the backups folder of noise's data folder, with the samples it hasn't saved; the newest ten of each song are kept, and File > Open Backup… opens one, to save under a name of its own. Backups keep their samples in one samples folder, named by their hashes as in a project, so each is written once.",
            ),
            P(
                "A song is saved as a project folder: Save As asks for its name, and the folder holds the song in project.json and, in samples, every sample it plays as a WAV file, whatever it was loaded from. Each sample's file is named by a hash of its audio, so the same audio is kept once however many samples play it, and saving deletes the samples nothing plays any more. The folder can be moved or copied as it is. Open (or the disk browser's Songs, which lists project folders as songs) opens a project folder, or a song saved on its own by an earlier version, which Save As then makes into a project folder.",
            ),
            P(
                "File > Export Project… writes the song and every sample it plays to a .noise file, to share or move: a zip archive of its project folder. File > Import Project…, Open, or a double-click in the disk browser's Songs unpacks one into a project folder beside it, named after the project (with a number if that name is taken), and opens it. Only the song and its samples come out of it, and nothing in it can reach outside that folder.",
            ),
            P(
                "File > Render to WAV… and Render Stems… first ask for the sample rate (the sound device's, or 22.05 to 96 kHz) and the format: 16 or 24 bit, dithered, or 32 bit float, which keeps peaks over 0 dB. The choice is remembered. Render Stems… renders each instrument to a WAV file of its own, soloed with its effects and its share of shared ones, all as long as the song, named after the file picked with the instrument's number and name. Renders, exports, imports and opening a song run in the background: the status bar shows how far they have got, with Cancel for renders, and the window keeps working meanwhile.",
            ),
            P(
                "From a terminal: noise song.json opens a song, noise --export song.json out.wav renders one without a window.",
            ),
            H("Preferences"),
            P(
                "Kept in settings.json in the config folder ($XDG_CONFIG_HOME/noise, usually ~/.config/noise): the MIDI input, which parts of the window and which scope show, the disk browser's folder and filter, and Follow. Presets you save are kept in the data folder ($XDG_DATA_HOME/noise, usually ~/.local/share/noise); factory presets are part of the program.",
            ),
            H("Song comments"),
            P(
                "View > Song Comments holds the song's title, artist and notes, saved with it; the title and artist show in the menu bar.",
            ),
        ],
    },
    Topic {
        title: "Pattern editor",
        blocks: &[
            P(
                "The cursor line stays in the middle and the pattern scrolls under it; the wheel moves the cursor. A red outline means edit mode is on: notes are written and the cursor moves on by STEP lines.",
            ),
            Code(
                "C-4 02 40 F8C\n│   │  │  └─ effect: command + hex argument\n│   │  └──── volume, 00–80, or a command such as O4\n│   └─────── module that plays the note (hex)\n└─────────── note",
            ),
            H("Tracks and columns"),
            P(
                "Each track header shows the name on its color, the mute button on its left and S on its right. Click the mute button to mute the track and right-click it to solo it; wherever a track shows (its header, the matrix's track headers, the track scopes), click mutes and right-click solos. Soloing plays that track alone until it is soloed again. Click a track's name to move to it, double-click to rename it, right-click it to pick a color, insert, delete or clear tracks.",
            ),
            P(
                "A track has up to eight note columns for chords and overlapping notes, and up to eight effect columns: the − and + under the track's name (left for note columns, right for effect columns), COLUMNS and FX above the pattern, or the track's menu add and remove them. An effect column holds a command that acts on every note column of the track, on top of each note column's own, so a line can have several. It can also show a panning column (00 left, 40 middle, 80 right) and a delay column (xx/256 of a line, in place of Dxx), with PAN and DLY.",
            ),
            H("Blocks"),
            P(
                "Select with Shift and the arrows, by dragging or with Shift+click; Ctrl+A selects the track, then the pattern. A block can be cut, copied, pasted, mix-pasted (only into empty fields), deleted, transposed, interpolated, humanized (each note's volume moved up to 10% at random, and up to an eighth of a line late in tracks showing their delay column), sent to the selected instrument, expanded or shrunk, or rendered to a sample, which plays it offline with its tails and loads it into a new Sampler. Without a selection these act on the cell under the cursor. They are in the Edit menu and the pattern's right-click menu. Copied blocks also go to the clipboard as text, to paste into another window.",
            ),
            H("Block loop"),
            P(
                "LOOP above the pattern repeats a stretch of lines while the song plays this slot: the size picked beside it around the cursor, or the selection, marked in green. Alt+L turns it on and off, Alt+Up and Alt+Down move it.",
            ),
            H("MIDI keyboard"),
            P(
                "Edit > MIDI Input… picks a keyboard. Its notes play the selected instrument at their velocity and, in edit mode, are written at the cursor with the velocity as the volume (unless switched off there).",
            ),
        ],
    },
    Topic {
        title: "Effect commands",
        blocks: &[
            Table(EFFECTS),
            H("Volume column"),
            P(
                "Besides a volume from 00 to 80, the volume column takes a command: a letter, typed in its first digit, and a hex digit. It acts like the effect it stands for, so the effect column stays free; a command in the effect column on the same line wins.",
            ),
            Table(VOL_COMMANDS),
            P(
                "Each line has TPL ticks, 6 unless changed. Effects work during their line: slides, glides, volume slides and panning stay afterwards, while arpeggio, vibrato, tremolo and auto-pan end with the line. Type J, L, N, R, S, T, W, Y or Z in the effect column for the commands written with letters. In phrases, Bxx, Fxx, Jxx, Lxx, Wxx and Zxx do nothing.",
            ),
        ],
    },
    Topic {
        title: "Sequencer and matrix",
        blocks: &[
            P(
                "The sequencer lists the song's slots under their sections' labels; each plays a pattern, its number. Drag a slot up or down to move it; Shift+drag its number to pick another pattern, or double-click the number to type one (one past the last makes a new pattern). In the extended view, double-click a pattern's name to rename it. Right-click a slot for all of these and the buttons' edits. The buttons down its left edit the list: + inserts a new pattern, Clone a copy of this one, Repeat plays it again, − removes the slot, and the arrows move it up or down. The last arrow switches to the extended view, which adds each slot's number, the pattern's name if it has one (names are optional: type one beside PATTERN over the editor) and the pattern matrix, with each track's color and name over its column.",
            ),
            H("Pattern matrix"),
            P(
                "A block for every track of every slot: filled in the track's color when it has notes there, outlined when empty. Click a block to edit that slot and track. Ctrl+click it, or use its menu, to mute the track in that slot only; it is crossed out and dimmed in the pattern editor.",
            ),
            P(
                "The colored columns above the matrix are its track headers, with the tracks' names: click one to mute the track, right-click to solo it. A block's menu mutes a track in every slot. Right-click a slot's name to mute or unmute every track there; Solo Here on a block mutes the slot's other tracks.",
            ),
            P(
                "Shift+click a second block to select a range. The right-click menu copies, cuts, pastes, clears or mutes the selected blocks, so tracks move between slots. Pasting into a pattern that plays in several slots changes it in all of them.",
            ),
            H("Sections"),
            P(
                "Right-click a slot's name and pick Add Section Here to name a part of the song. Click a section to select its blocks, double-click to rename it, right-click to remove it.",
            ),
        ],
    },
    Topic {
        title: "Mixer",
        blocks: &[
            P(
                "Sound flows through modules rather than tracks, so the mixer (F3) has a strip for every module: instruments, then effects, with the output as the master strip on the right.",
            ),
            P(
                "Each strip shows the module's name (click to select it), where its sound goes, pan, a fader from -inf to +6 dB (double-click for 0 dB), stereo meters, and mute and solo. Fader and pan act after the module's own settings. Soloing a module keeps it, what feeds it and what it feeds audible.",
            ),
        ],
    },
    Topic {
        title: "Instrument editor",
        blocks: &[
            P(
                "The Instrument tab (F4) edits the selected instrument. Its device chain shows everything its sound goes through as panels: a synth's page is Synth, a Sampler's is Effects, beside its Waveform, Keyzones, Modulation and Phrase pages.",
            ),
            List(&[
                "The instrument comes first; its panel mutes and solos it.",
                "Its own effects follow, in order. Each can be switched off, moved by dragging its name or with the left and right arrows, or removed with the bin. + Add Effect adds one at the end.",
                "Sends to lists where the sound goes next: the output, or effects other instruments share. Tick several to split it.",
                "Shared effects come next, marked shared; the bin deletes one for every instrument.",
                "The Modulators moving any of these come last; + Add Modulator adds one.",
                "A MultiSynth's panel lists the instruments it plays.",
            ]),
            H("Macros"),
            P(
                "Every instrument has eight macros, in the panel left of its chain: knobs that each move any number of parameters of the instrument and its own effects at once. Right-click a parameter's bar and choose Map to Macro: the macro then moves it from where it is to the far end of its range, which you can change by right-clicking its line under the macro. Right-click a macro to name it, automate it or clear what it moves. Envelopes and Modulators move macros as they do any parameter, and instrument presets keep them.",
            ),
            H("Presets"),
            P(
                "Right-click a panel's name, or a row of the module list, for its color and presets: the factory's (the demo songs' devices), yours, saving the settings under a name, or going back to the defaults. Yours are kept in presets/<kind>/ in the data folder (usually ~/.local/share/noise).",
            ),
            H("Instrument presets"),
            P(
                "The instrument list's + adds a whole instrument from Factory Instruments, the demo songs' (Felt Piano, Strings, Sub Bass, Bells, 808, Chip Arp, Vox Chops, Rain Break, Acid Line, Music Box, Rave Kit, Prism Lead, Orchestra Hit and the rest), or from My Instruments. An instrument preset holds the instrument with its samples, modulation, macros and phrases, and the effects of its own chain. Right-click an instrument and Save as Preset to keep it in instruments/ in the data folder, its samples as WAV files beside it.",
            ),
        ],
    },
    Topic {
        title: "Phrases",
        blocks: &[
            P(
                "Every instrument has a Phrase page with short patterns of its own: up to 64 lines of notes, volumes and effects each, at their own LPB and the song's BPM. +, Dup and − add, copy and delete phrases; the numbered buttons pick the one shown.",
            ),
            P(
                "A note that plays a phrase plays it instead, transposed so the phrase's C-4 is the note played, with the note's velocity. The note-off stops it; Loop starts it again after its last line while the note is held.",
            ),
            H("Which phrase plays"),
            List(&[
                "PLAY Off: notes play the instrument itself.",
                "Program: every note plays the phrase shown.",
                "Keymap: a note plays the phrase whose KEYS hold it, or the instrument outside them.",
                "Zxx in the pattern picks phrase xx for its note whatever the mode; Z00 plays none.",
            ]),
            H("Editing"),
            P(
                "In edit mode (Esc), type notes, hex volumes and effects as in the pattern: A or 1 for a note-off, Del to clear, arrows to move. Effects play on the phrase's own ticks, except Bxx, Fxx and Zxx. While phrases play, the line each note has reached is lit, and a dot under a phrase's number shows it is playing.",
            ),
        ],
    },
    Topic {
        title: "Modules",
        blocks: &[
            Table(MODULES),
            P(
                "The lower frame's Modules tab lists every module as a table: instruments, effects, MultiSynths and Modulators, and the output last. Each row has its color, number, name, kind, level, where it sends, and mute, solo and off switches. Click a row to edit it on the right, with what feeds it and where it sends as lists to tick. Double-click an instrument to hear it; right-click a row for its color, to duplicate or delete it. + Add adds any module.",
            ),
            H("Parameters"),
            P(
                "Parameters are bars with their name and value inside. Drag one to change it (Shift for fine steps), double-click to reset it, right-click to type a value or automate it. Frequencies move in octaves.",
            ),
            H("MultiSynth"),
            P(
                "It plays no sound itself: it passes its notes on to every instrument it is connected to, so one track plays layered instruments. It can transpose, finetune, detune each note at random, scale velocity, ignore notes outside a range, and send each note to all its instruments, to the next in turn (Round robin) or to one at random.",
            ),
            H("Glide"),
            P(
                "A note module, connected to instruments as a MultiSynth is: each note it passes on starts where the last one on its key (its column, for a track) was and slides to its own pitch in Time, near or far. In Legato mode only a note played over the last one, as a track does with no note-off between, slides, and it goes on sounding rather than starting again; a note after a gap jumps. Slides and vibrato on the note still apply.",
            ),
            H("Track effects and the master chain"),
            P(
                "Each track can have effects of its own: what it plays goes through them after each instrument's own effects, then on to the master. Open them with the lower frame's Track FX tab, the FX badge on a track's header (shown once it has any) or Track Effects… in the header's menu. The tab's chips pick a track, in its colors, and follow the pattern cursor; the last chip is the master chain, the effects the whole mix goes through before the output.",
            ),
            P(
                "+ Add Effect adds one at the end; drag one by its name, or use its arrows, to move it; the bin removes it. Modulators and automation move them like any other effect, and the mixer shows a strip for each. A track with effects shows their sound in its track scope.",
            ),
            H("Groups"),
            P(
                "A track can go through another track's effects before the master, as a bus: pick that track in the Group submenu of the track's header menu. Its sound goes through its own effects, if it has any, then the group's, so several tracks share a compressor or reverb and one fader. A group can be in a group of its own; a track can't go through one whose sound already comes through it. The end of a grouped track's chain in the Track FX tab says which group it goes to, with Show Group to open its effects.",
            ),
            P(
                "An instrument played on several tracks keeps them apart: each track with effects gets its own copy of the instrument and its own effects, so a delay on the hats track echoes the hats but not the kicks the same drum kit plays on another track. Tracks without effects share the one instrument, at no cost.",
            ),
            H("Modulator"),
            P(
                "A controller: its links move parameters (or a mixer fader or pan) instead of carrying sound. As an LFO it swings them with one of five shapes, or one you draw (Drawn: click to add points, drag to move them, right-click to remove them), in Hz for a constant time, or synced to a period in lines or a length in beats from 1/16 to 32; only the settings the mode uses are shown; set to Follow input it raises them with the level of the sound fed into it, for sidechain effects. In Key and Velocity modes it moves them by the last note played on the instruments it follows: up for notes above C-4 and down for those below, or by how hard the note was played. In Envelope mode each note on them starts an envelope that rises over the attack and falls over the release; in Manual mode its Amount alone moves every parameter it is linked to, as one knob for them all. Amount sets how far; negative moves the other way.",
            ),
            H("Repeater"),
            P(
                "While Hold is on it plays the stretch of input just before it came on, Length long, over and over, in time with the song. Automate Hold, or switch it from a Modulator, for stutters and beat repeats.",
            ),
            H("Vocal Filter"),
            P(
                "Vowel runs through A, E, I, O and U; automate it, or move it with a Modulator, to make a sound talk. Shift moves the formants by semitones, Width scales their bandwidths.",
            ),
        ],
    },
    Topic {
        title: "Automation",
        blocks: &[
            P(
                "A pattern can carry envelopes that move module parameters while it plays. The lower frame's Automation tab shows the current pattern's automated parameters, or under All every module's, grouped and folded, with a filter to find one by its module's or its own name; a dot marks the automated ones. Pick a parameter and click the graph to start its envelope, or right-click a parameter bar and choose Automate. An envelope takes the whole pattern or, with Length, a half, a quarter, an eighth or a sixteenth of it; a shorter one holds its last value after it ends, or with Repeat starts again through the rest of the pattern.",
            ),
            P(
                "Lines run left to right and the parameter's range bottom to top. Click to add a point, drag a point to move it, drag elsewhere to draw, right-click a point to delete it. Points snap to lines unless Shift is held. Points holds each value until the next, Lines moves straight between them, Curve smoothly.",
            ),
            P(
                "Automated parameters are marked in the parameter list. While the song plays their bars and the mixer follow the envelopes; the song's own values come back when it stops.",
            ),
        ],
    },
    Topic {
        title: "Sampler",
        blocks: &[
            P(
                "The Sampler holds samples, each with its own settings and a keyzone: the notes and velocities that play it. Select it and press F4.",
            ),
            H("Loading"),
            P(
                "Drop WAV, FLAC or Ogg Vorbis files on the window, double-click them in the disk browser, or press Load…. If the selected instrument isn't a Sampler, a new one is made. Several files at once become a drum kit, a key each from C-4.",
            ),
            H("Recording"),
            P(
                "Record… opens the Sample Recorder. It records the sound device's input (a microphone or line in), or the song's output to make a sample of it (resampling): from the start of the song, stopping with it, or whatever plays until you press Stop. Silence before the sound starts is trimmed unless you say otherwise, and the recording becomes a new sample of the selected Sampler, saved next to the song.",
            ),
            H("Soundfonts"),
            P(
                "SF2, SF3 and SFZ files load the same way and replace the Sampler's samples with theirs: keys, velocities, root notes, tuning, volume, panning and loops, with the envelope as the Sampler's. A file with several presets opens a list to pick one from. SF2 exclusive classes become mute groups; modulators, filters and LFOs are left out.",
            ),
            H("Waveform"),
            List(&[
                "Drag to select; the wheel zooms, the bar below scrolls.",
                "Reverse, Normalize, Fade in and out, Silence and the Process menu change the selection, or the whole sample. Crossfade Loop blends the loop's end into its start.",
                "Process > Time Stretch makes the selection, or the whole sample, longer or shorter without changing its pitch: by a ratio, or to fit 1 to 64 lines at the song's tempo, as a loop to the beat. The loop and slice markers move with it.",
                "Crop, Delete, Cut, Copy and Paste work on the selection; Loop selection makes it the loop. The play button plays it.",
                "Below: volume, panning, base note, transpose, finetune, loop mode and points, and the keyzone. Drag the loop markers to move the loop.",
                "Beat sync plays the sample in a number of lines at the song's tempo. One-shot ignores note-offs. Autoseek plays the sample from where it would be when the song starts partway through, for long loops and vocals; without it a sample waits for its next note. A mute group cuts the others in it, as a closed hi-hat an open one.",
            ]),
            H("Slicer"),
            P(
                "Double-click the strip above the waveform to add a slice marker, drag one to move it, right-click to delete it. Slices divides the sample evenly, detects beats, clears markers or renders each slice to a sample. A sliced sample plays whole on its base note and slice 1, 2, 3… on the notes above. Select a slice in the sample list to give it its own volume, panning, tuning, loop or one-shot.",
            ),
            H("Keyzones"),
            P(
                "Samples across the keyboard (left to right) and velocity (bottom to top). Drag a zone or its edges; click the keyboard to play. Overlapping zones play together. Layer all, Drum kit and Spread lay them all out.",
            ),
            H("Modulation"),
            P(
                "Changes every voice while it plays: a pitch envelope, a filter with a cutoff envelope, key tracking (higher notes open it, lower ones close it, from C-4) and velocity (softer notes close it by up to so many octaves), and vibrato and tremolo LFOs. Envelopes run in seconds from the note's start: click to add a point, drag it, right-click to delete it, double-click to make it the sustain point. Generators and FM have the same page.",
            ),
            H("Granular"),
            P(
                "A Granular instrument loads and edits samples in the same editor. A note plays the first sample whose keyzone holds it as a cloud of grains, at its pitch from the sample's base note, transpose and finetune, with the sample's volume and panning; loops, slices and the Sampler's other settings don't apply. Position is where grains start, Scan how fast that point moves through the sample (1 at the sample's own speed, 0 frozen, below 0 backwards), and Spray scatters each grain around it. Size and Density set how long grains are and how many start a second; Random pitch, Stereo and Reverse vary each one.",
            ),
        ],
    },
    Topic { title: "Keys", blocks: &[Table(KEYS)] },
];

/// The manual's open topic and search.
#[derive(Default)]
pub struct HelpView {
    topic: usize,
    find: String,
}

impl HelpView {
    /// Shows the topic titled `title`.
    pub fn open(&mut self, title: &str) {
        self.find.clear();
        self.topic = TOPICS.iter().position(|t| t.title == title).unwrap_or(0);
    }
}

/// The text of a block, for searching.
fn words(block: &Block) -> String {
    match block {
        P(t) | H(t) | Code(t) => t.to_string(),
        List(items) => items.join(" "),
        Table(rows) => rows.iter().map(|(a, b)| format!("{a} {b}")).collect::<Vec<_>>().join(" "),
    }
}

fn matches(topic: &Topic, find: &str) -> bool {
    find.is_empty()
        || topic.title.to_lowercase().contains(find)
        || topic.blocks.iter().any(|b| words(b).to_lowercase().contains(find))
}

pub fn window(ctx: &egui::Context, open: &mut bool, view: &mut HelpView) {
    let window = egui::Window::new("Manual")
        .open(open)
        .default_size([720.0, 600.0])
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    window.show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("noise {}", env!("CARGO_PKG_VERSION"))).color(theme::SELECTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut view.find).desired_width(200.0).hint_text("Find in the manual"));
            });
        });
        ui.separator();
        let find = view.find.trim().to_lowercase();
        let shown: Vec<usize> = (0..TOPICS.len()).filter(|&i| matches(&TOPICS[i], &find)).collect();
        if !shown.contains(&view.topic)
            && let Some(&first) = shown.first()
        {
            view.topic = first;
        }
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                for &i in &shown {
                    if theme::toggle(ui, view.topic == i, TOPICS[i].title).clicked() {
                        view.topic = i;
                    }
                }
                if shown.is_empty() {
                    ui.label(RichText::new("Nothing found.").color(theme::TEXT_WEAK));
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt(view.topic).auto_shrink(false).show(ui, |ui| {
                // Inside the row, the topic would run on sideways.
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width());
                    let Some(topic) = TOPICS.get(view.topic).filter(|_| !shown.is_empty()) else { return };
                    ui.label(RichText::new(topic.title).heading().color(theme::SELECTED));
                    for (k, block) in topic.blocks.iter().enumerate() {
                        show(ui, block, k);
                    }
                });
            });
        });
    });
}

fn show(ui: &mut egui::Ui, block: &Block, k: usize) {
    match block {
        P(text) => {
            ui.add_space(4.0);
            ui.label(*text);
        }
        H(text) => {
            ui.add_space(8.0);
            theme::caption(ui, &text.to_uppercase());
        }
        List(items) => {
            ui.add_space(4.0);
            for item in *items {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("•").color(theme::SELECTED));
                    ui.add(egui::Label::new(*item).wrap());
                });
            }
        }
        Table(rows) => {
            ui.add_space(4.0);
            // A column grows to its widest text unless held to what the
            // window leaves beside the keys.
            let most = (ui.available_width() - 230.0).max(200.0);
            let grid = egui::Grid::new(("help_table", k)).num_columns(2).striped(true).spacing([16.0, 3.0]);
            grid.max_col_width(most).show(ui, |ui| {
                for (key, what) in *rows {
                    ui.label(RichText::new(*key).monospace().color(theme::PAT_INSTRUMENT));
                    ui.add(egui::Label::new(*what).wrap());
                    ui.end_row();
                }
            });
        }
        Code(text) => {
            ui.add_space(4.0);
            egui::Frame::new().fill(theme::INSET).inner_margin(6).corner_radius(2).show(ui, |ui| {
                ui.label(RichText::new(*text).monospace().color(theme::PAT_NOTE));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_command_and_module_is_in_the_manual() {
        // The effect commands the engine knows, and every module that can
        // be added, each have a row.
        for cmd in [
            "0xy", "1xx", "3xx", "4xy", "7xy", "8xx", "9xx", "Axy", "Bxx", "Cxx", "Dxx", "Exx", "Fxx", "Jxx", "Lxx",
            "Nxy", "Rxx", "Sxx", "Txy", "Wxx", "Yxx", "Zxx",
        ] {
            assert!(EFFECTS.iter().any(|(k, _)| k.contains(cmd)), "{cmd}");
        }
        for c in crate::project::VOL_COMMANDS {
            assert!(VOL_COMMANDS.iter().any(|(k, _)| k.contains(&format!("{c}x"))), "{c}x");
        }
        for kind in crate::project::ModuleKind::ADDABLE {
            assert!(MODULES.iter().any(|(k, _)| *k == kind.name()), "{}", kind.name());
        }
    }

    #[test]
    fn search_finds_topics_by_their_text() {
        let found: Vec<&str> = TOPICS.iter().filter(|t| matches(t, "soundfont")).map(|t| t.title).collect();
        assert!(found.contains(&"Sampler"));
        assert!(!found.contains(&"Mixer"));
        assert_eq!(TOPICS.iter().filter(|t| matches(t, "")).count(), TOPICS.len());
    }
}
