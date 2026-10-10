//! Rendering a block: envelopes and Modulators moving parameters, macros,
//! each module in turn, the mixer and what the window is shown.

use super::*;

impl Engine {
    /// Sets the parameters that the playing pattern's envelopes control,
    /// for the block about to be rendered.
    pub(super) fn automate(&mut self) {
        for node in &mut self.nodes {
            node.automated = false;
            node.mix_gain = None;
            node.mix_pan = None;
            node.macros = [None; MACROS];
        }
        self.live.clear();
        if !self.playing {
            return;
        }
        let project = self.project.clone();
        let Some(pattern) = project.order.get(self.shown.0).and_then(|s| project.patterns.get(s.pattern)) else {
            return;
        };
        let pos = self.pattern_line() as f32;
        for env in &pattern.automation {
            let Some(module) = project.module(env.module) else { continue };
            let (Some(spec), Some(t)) = (module.kind.automatable(env.param), env.value_in(pos, pattern.lines)) else {
                continue;
            };
            let value = spec.value_at(t);
            // The module and its copies for tracks with effects.
            for node in self.nodes.iter_mut().filter(|n| n.id == env.module) {
                node.set_param(module, env.param, value);
            }
            if self.live.len() < MAX_AUTOMATED {
                self.live.push((env.module, env.param, value));
            }
        }
    }

    /// Runs Modulator `i` for a block of `frames` and moves the parameters
    /// it controls, on top of the song's value or the envelope's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn modulate(
        nodes: &mut [Node],
        live: &mut Vec<(u8, usize, f32)>,
        project: &Project,
        ctx: &Ctx,
        i: usize,
        frames: usize,
        input_peak: f32,
    ) {
        let node = &nodes[i];
        let module = &project.modules[node.module];
        let p = if node.automated { &node.params } else { &module.params };
        let (mode, shape, rate, sync, period, amount, attack, release) =
            (p[0].round() as u32, p[1].round() as u32, p[2], p[3].round() as u32, p[4], p[5], p[6], p[7]);
        let beats = crate::project::MODULATOR_BEAT_LENGTHS[(p[8].round() as usize).min(9)];
        // The last note played on what it follows, for Key and Velocity.
        let last = || {
            let notes = nodes[i].inputs.iter().map(|&j| nodes[j].last_note);
            notes.filter(|n| n.2 > 0).max_by_key(|n| n.2)
        };
        let followed = match mode {
            0 => None,
            1 => Some(input_peak.min(1.0)),
            // Up or down from C-4 by up to four octaves.
            2 => Some(last().map_or(0.0, |n| ((n.0 - 48.0) / 48.0).clamp(-1.0, 1.0))),
            3 => Some(last().map_or(0.0, |n| n.1)),
            // A new note starts the envelope: up over the attack, then down
            // over the release. `phase` counts the seconds since it began.
            4 => {
                let started = last().map_or(0, |n| n.2);
                let node = &mut nodes[i];
                if started != node.last_note.2 {
                    node.last_note.2 = started;
                    node.phase = 0.0;
                } else {
                    node.phase += frames as f32 / ctx.sr;
                }
                let up = (node.phase / attack.max(1e-4)).min(1.0);
                let down = ((node.phase - attack).max(0.0) / release.max(1e-4)).min(1.0);
                // The envelope itself moves; it skips the follower.
                node.follow = if started == 0 { 0.0 } else { up * (1.0 - down) };
                None
            }
            _ => Some(1.0),
        };
        let node = &mut nodes[i];
        let offset = if mode == 4 {
            amount * node.follow
        } else if let Some(target) = followed {
            // Followed through the attack and release, without jumps.
            let time = if target > node.follow { attack } else { release };
            let coef = 1.0 - (-(frames as f32) / (time.max(1e-4) * ctx.sr)).exp();
            node.follow += coef * (target - node.follow);
            amount * node.follow
        } else {
            let lines = match sync {
                0 => None,
                1 => Some(period),
                _ => Some(beats * project.lpb.max(1) as f32),
            };
            // Synced, its cycle follows the song while it plays.
            if let (Some(l), Some(at)) = (lines, ctx.song_line) {
                node.phase = (at / l.max(1e-3) as f64).rem_euclid(1.0) as f32;
            }
            let inc = match lines {
                Some(l) => frames as f32 / (l * ctx.samples_per_line).max(1.0),
                None => rate * frames as f32 / ctx.sr,
            };
            let w = if shape == crate::project::DRAWN_SHAPE {
                let points =
                    if module.shape.is_empty() { &crate::project::DEFAULT_SHAPE[..] } else { &module.shape[..] };
                crate::project::interpolate(points, node.phase, false, false).map_or(0.0, |v| 2.0 * v - 1.0)
            } else {
                dsp::lfo_shape(shape, node.phase)
            };
            node.phase = (node.phase + inc).fract();
            amount * w * 0.5
        };
        if module.mute {
            return;
        }
        for k in 0..nodes[i].controls.len() {
            let (j, param) = nodes[i].controls[k];
            let target = &mut nodes[j];
            let m = &project.modules[target.module];
            let Some(spec) = m.kind.automatable(param) else { continue };
            let value = spec.value_at(spec.position(target.param(m, param)) + offset);
            target.set_param(m, param, value);
            if live.len() < MAX_AUTOMATED {
                live.push((m.id, param, value));
            }
        }
    }

    pub(super) fn render_block(&mut self, out: &mut [Frame]) {
        let n = out.len();
        let samples_per_line = (self.samples_per_tick() * self.tpl.max(1) as f64) as f32;
        let ctx = Ctx { sr: self.sr, samples_per_line, song_line: self.song_line() };
        let project = &self.project;
        for oi in 0..self.order.len() {
            let i = self.order[oi];
            let scratch = &mut self.scratch[..n];
            mix(scratch, &self.nodes, &self.nodes[i].inputs);
            if !self.nodes[i].key.is_empty() {
                mix(&mut self.key_scratch[..n], &self.nodes, &self.nodes[i].key);
            }
            // An instrument's macros set what they move before it plays.
            for k in 0..self.nodes[i].macro_targets.len() {
                let (mac, j, param, from, to) = self.nodes[i].macro_targets[k];
                let m = &project.modules[self.nodes[i].module];
                let turned = self.nodes[i].param(m, m.params.len() + 2 + mac);
                let target = &project.modules[self.nodes[j].module];
                let Some(spec) = target.kind.automatable(param) else { continue };
                let value = spec.value_at(from + (to - from) * turned);
                self.nodes[j].set_param(target, param, value);
                if self.live.len() < MAX_AUTOMATED {
                    self.live.push((target.id, param, value));
                }
            }
            if self.nodes[i].kind.controls() {
                let input_peak = scratch.iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
                Self::modulate(&mut self.nodes, &mut self.live, project, &ctx, i, n, input_peak);
            }
            let node = &mut self.nodes[i];
            let module = &project.modules[node.module];
            let params = if node.automated { &node.params } else { &module.params };
            if module.bypass && module.kind.has_input() {
                // A switched-off effect lets its input through.
                node.buf[..n].copy_from_slice(scratch);
            } else if node.key.is_empty() {
                node.dsp.process(&ctx, params, scratch, &mut node.buf[..n]);
            } else {
                node.dsp.process_keyed(&ctx, params, scratch, &self.key_scratch[..n], &mut node.buf[..n]);
            }
            let silent = module.mute || !node.audible;
            if let Some(tap) = node.dsp.track_tap() {
                if !silent {
                    let mut used = tap.used;
                    while used != 0 {
                        let t = used.trailing_zeros() as usize;
                        for (sum, x) in self.track_block[t][..n].iter_mut().zip(&tap.sums[t][..n]) {
                            *sum += x;
                        }
                        used &= used - 1;
                    }
                }
                tap.clear();
            }
            let buf = &mut node.buf[..n];
            let (gain, pan) = (node.mix_gain.unwrap_or(module.gain), node.mix_pan.unwrap_or(module.pan));
            if module.mute || !node.audible {
                buf.fill([0.0; 2]);
            } else if gain != 1.0 || pan != 0.0 {
                let (l, r) = balance(pan);
                for f in buf.iter_mut() {
                    f[0] *= gain * l;
                    f[1] *= gain * r;
                }
            }
            for f in buf.iter() {
                node.peak[0] = node.peak[0].max(f[0].abs());
                node.peak[1] = node.peak[1].max(f[1].abs());
            }
        }
        match self.output {
            Some(o) => out.copy_from_slice(&self.nodes[o].buf[..n]),
            None => out.fill([0.0; 2]),
        }
        // A track with effects shows them in its scope: their sound.
        for &(t, j) in &self.track_ends {
            for (s, f) in self.track_block[t][..n].iter_mut().zip(&self.nodes[j].buf[..n]) {
                *s = (f[0] + f[1]) * 0.5;
            }
        }
        for k in 0..n {
            for (t, block) in self.track_block.iter().enumerate() {
                self.track_rings[t * TRACK_SCOPE_LEN + self.track_pos] = block[k];
            }
            self.track_pos = (self.track_pos + 1) % TRACK_SCOPE_LEN;
        }
        for block in &mut self.track_block {
            block[..n].fill(0.0);
        }
    }

    /// Renders `out.len()` frames, running the sequencer on tick boundaries.
    pub fn render(&mut self, out: &mut [Frame]) {
        let started = std::time::Instant::now();
        crate::dsp::flush_denormals();
        if let Some(rx) = self.rx.take() {
            while let Ok(cmd) = rx.try_recv() {
                self.handle(cmd);
            }
            self.rx = Some(rx);
        }

        let mut done = 0;
        while done < out.len() {
            if self.playing && self.samples_to_tick <= 0.0 {
                self.run_tick();
                self.samples_to_tick += self.tick_wait();
            }
            let mut n = (out.len() - done).min(BLOCK);
            if self.playing {
                n = n.min(self.samples_to_tick.ceil().max(1.0) as usize);
            }
            if self.gliding() {
                n = n.min(GLIDE_STEP);
            }
            self.automate();
            self.run_phrases(n);
            self.run_glides(n);
            self.render_block(&mut out[done..done + n]);
            self.render_preview(&mut out[done..done + n]);
            self.render_click(&mut out[done..done + n]);
            if self.playing {
                self.samples_to_tick -= n as f64;
                self.played += n as u64;
            }
            done += n;
        }

        let mut peak = [0f32; 2];
        for f in out.iter_mut() {
            for ch in 0..2 {
                // Guard against blowups from extreme parameter settings.
                if !f[ch].is_finite() {
                    f[ch] = 0.0;
                }
                peak[ch] = peak[ch].max(f[ch].abs());
            }
            self.scope[self.scope_pos] = *f;
            self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
        }
        self.shared.resample.push(out);
        let load = started.elapsed().as_secs_f32() * self.sr / out.len().max(1) as f32;
        self.publish(peak, load, out.len());
    }

    pub(super) fn publish(&mut self, peak: [f32; 2], load: f32, frames: usize) {
        let s = &self.shared;
        let fall = (-(frames as f32) / (self.sr * METER_FALL)).exp();
        // A module's meter shows the loudest of it and its copies.
        let mut levels = [[0f32; 2]; 256];
        for node in &mut self.nodes {
            for (l, peak) in levels[node.id as usize].iter_mut().zip(&mut node.peak) {
                let level = if peak.is_finite() { *peak } else { 0.0 };
                *l = l.max(level);
                *peak = level * fall;
            }
        }
        for node in self.nodes.iter().filter(|n| n.track.is_none()) {
            let at = 2 * node.id as usize;
            for (out, l) in s.levels[at..at + 2].iter().zip(levels[node.id as usize]) {
                out.store(l.to_bits(), Ordering::Relaxed);
            }
        }
        let time = (self.played as f64 / self.sr as f64) as f32;
        s.time.store(time.to_bits(), Ordering::Relaxed);
        let cpu = f32::from_bits(s.cpu.load(Ordering::Relaxed));
        s.cpu.store((cpu + (load - cpu) * 0.05).to_bits(), Ordering::Relaxed);
        s.playing.store(self.playing, Ordering::Relaxed);
        s.order.store(self.shown.0, Ordering::Relaxed);
        s.line.store(self.shown.1, Ordering::Relaxed);
        let into_tick = (1.0 - self.samples_to_tick / self.samples_per_tick()).clamp(0.0, 1.0) as f32;
        let frac = ((self.shown_tick as f32 + into_tick) / self.tpl.max(1) as f32).clamp(0.0, 0.999);
        s.line_frac.store(frac.to_bits(), Ordering::Relaxed);
        s.bpm.store(self.bpm.to_bits(), Ordering::Relaxed);
        for (p, out) in peak.iter().zip(&s.peak) {
            let old = f32::from_bits(out.load(Ordering::Relaxed));
            out.store(p.max(old * 0.9).to_bits(), Ordering::Relaxed);
        }
        if let Ok(mut scope) = s.scope.try_lock() {
            let (a, b) = self.scope.split_at(self.scope_pos);
            scope[..b.len()].copy_from_slice(b);
            scope[b.len()..].copy_from_slice(a);
        }
        let preview = self.preview.as_ref().map_or(-1.0, |p| p.1 as f32);
        s.preview_pos.store(preview.to_bits(), Ordering::Relaxed);

        self.playheads.clear();
        for n in self.nodes.iter().filter(|n| n.kind == ModuleKind::Sampler) {
            let from = self.playheads.len();
            n.dsp.playheads(&mut self.playheads);
            for p in &mut self.playheads[from..] {
                p.module = n.id;
            }
        }
        self.playheads.truncate(MAX_PLAYHEADS);
        if let Ok(mut out) = s.playheads.try_lock() {
            out.clear();
            out.extend_from_slice(&self.playheads);
        }
        if let Ok(mut out) = s.track_scopes.try_lock() {
            let p = self.track_pos;
            for (dst, src) in out.chunks_mut(TRACK_SCOPE_LEN).zip(self.track_rings.chunks(TRACK_SCOPE_LEN)) {
                dst[..TRACK_SCOPE_LEN - p].copy_from_slice(&src[p..]);
                dst[TRACK_SCOPE_LEN - p..].copy_from_slice(&src[..p]);
            }
        }
        if let Ok(mut out) = s.automated.try_lock() {
            out.clear();
            out.extend_from_slice(&self.live);
        }
        if let Ok(mut out) = s.phrases.try_lock() {
            out.clear();
            out.extend(self.phrases.iter().map(|p| (p.module, p.phrase, p.playing)));
        }
    }
}

/// Left and right gains that turn a stereo signal towards `pan` (-1..1)
/// without making the near side louder.
fn balance(pan: f32) -> (f32, f32) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}
