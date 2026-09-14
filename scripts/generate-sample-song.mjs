// Generates a short synthetic "song" WAV file for the diagnostics app's
// native (non-browser) audio playback test — see MainActivity.kt's
// "Music File" section. Deliberately synthetic rather than a real
// downloaded track: no licensing question, fully reproducible, and this
// only needs to exercise the same AudioTrack/AudioFlinger/virtio-snd path
// a real song would, not actually sound good. Unlike the app's existing
// single-frequency test tone, this has a changing melody with two-partial
// "notes" (a fundamental + a harmonic) and per-note fade envelopes, closer
// in character to real music than a pure sine wave, while staying raw PCM
// (WAV) so this exercises the audio *output* pipeline specifically, not an
// unrelated codec decoder.
//
// Run: node scripts/generate-sample-song.mjs
// Regenerate only if the sample itself needs to change (longer, different
// melody, etc.) — the output is committed as a binary resource, same as
// resources/beo-diagnostics.apk.

import { writeFileSync } from "node:fs";
import { join } from "node:path";

const repoRoot = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const outPath = join(repoRoot, "diagnostics-app", "app", "src", "main", "res", "raw", "sample_song.wav");

const SAMPLE_RATE = 22050;

// A short melody (Hz, beat count) — nothing copyrighted, just a simple
// rising-and-falling scale-like sequence so the waveform actually changes
// over time instead of looping one tone.
const NOTES = [
  [261.63, 1], [293.66, 1], [329.63, 1], [349.23, 1],
  [392.0, 1], [440.0, 1], [493.88, 1], [523.25, 2],
  [493.88, 1], [440.0, 1], [392.0, 1], [349.23, 1],
  [329.63, 1], [293.66, 1], [261.63, 2],
];
const BEAT_SECONDS = 0.35;

const samples = [];
for (const [freq, beats] of NOTES) {
  const durationSec = beats * BEAT_SECONDS;
  const n = Math.round(durationSec * SAMPLE_RATE);
  for (let i = 0; i < n; i++) {
    const t = i / SAMPLE_RATE;
    // Fundamental + a quieter harmonic, so it's not a pure sine — closer
    // to a real instrument's timbre than the app's existing tone test.
    let v = Math.sin(2 * Math.PI * freq * t) * 0.7 + Math.sin(2 * Math.PI * freq * 2 * t) * 0.25;
    // Short fade-in/out per note to avoid clicks at note boundaries —
    // those would be artifacts of *this generator*, not the emulator's
    // audio path, and would contaminate the glitch signal this is for.
    const fadeSamples = Math.min(Math.round(0.02 * SAMPLE_RATE), Math.floor(n / 2));
    if (i < fadeSamples) v *= i / fadeSamples;
    else if (i > n - fadeSamples) v *= (n - i) / fadeSamples;
    samples.push(Math.max(-1, Math.min(1, v)));
  }
}

const numSamples = samples.length;
const dataSize = numSamples * 2; // 16-bit mono
const buffer = Buffer.alloc(44 + dataSize);

buffer.write("RIFF", 0, "ascii");
buffer.writeUInt32LE(36 + dataSize, 4);
buffer.write("WAVE", 8, "ascii");
buffer.write("fmt ", 12, "ascii");
buffer.writeUInt32LE(16, 16); // fmt chunk size
buffer.writeUInt16LE(1, 20); // PCM
buffer.writeUInt16LE(1, 22); // mono
buffer.writeUInt32LE(SAMPLE_RATE, 24);
buffer.writeUInt32LE(SAMPLE_RATE * 2, 28); // byte rate
buffer.writeUInt16LE(2, 32); // block align
buffer.writeUInt16LE(16, 34); // bits per sample
buffer.write("data", 36, "ascii");
buffer.writeUInt32LE(dataSize, 40);

for (let i = 0; i < numSamples; i++) {
  buffer.writeInt16LE(Math.round(samples[i] * 32767), 44 + i * 2);
}

writeFileSync(outPath, buffer);
console.log(`wrote ${outPath} (${(buffer.length / 1024).toFixed(1)} KB, ${(numSamples / SAMPLE_RATE).toFixed(1)}s)`);
