// Real-content audio glitch test: launches a real device through the real
// compiled Beo app (over CDP, same technique as e2e-avd-sideload.mjs), opens
// a live YouTube stream in Chrome, and checks the guest's own audio log for
// objective glitch/underrun signatures during playback.
//
// This exists because "does the audio still pop" kept being answered by
// subjective impression alone. It's not a substitute for actually
// listening — Claude/this script has no way to hear the output — but it
// gives a real, repeatable, objective signal (AudioFlinger buffer-strategy
// warnings, underrun/xrun log lines) to check alongside listening, and
// removes the manual click-through of Chrome's first-run flow + YouTube's
// cookie dialog every time this needs re-testing.
//
// This is one half of a two-part suite (see test-audio-file.mjs for the
// other): browser/WebView playback here, native MediaPlayer playback there,
// specifically so a glitch can be checked against both — Chrome's own
// autoplay/decode path could be the culprit, or it could be systemic to the
// emulator's audio pipeline regardless of what's playing. Shared scaffolding
// (build/launch/CDP, AVD create+boot, adb helpers, logcat analysis) lives in
// scripts/lib/audio-test-common.mjs.
//
// Deliberately local-only, not wired into CI — same reasoning as the other
// e2e scripts (real emulator boot, real network access to YouTube).
//
// Fragile by nature: Chrome's first-run flow and YouTube's cookie dialog
// are matched by button *text*, not fixed coordinates, which survives
// screen-density/layout changes better than coordinates but will still
// break if Google changes the wording or adds/removes a step. If this
// script starts failing to get past onboarding, that's the first thing to
// check — not necessarily a real regression in Beo or the emulator.
//
// Usage: node scripts/test-audio-youtube.mjs [--skip-build]

import { setTimeout as sleep } from "node:timers/promises";
import {
  adb,
  captureAndAnalyzeLogcat,
  captureDebugState,
  cleanupAvd,
  connectToBeo,
  createAndBootAvd,
  dumpUi,
  hasAudioFocus,
  makeLogger,
  tapText,
} from "./lib/audio-test-common.mjs";

const skipBuild = process.argv.includes("--skip-build");
const CDP_PORT = 9335;
const DEVICE = "audio_glitch_test";
// A specific video id (the original approach here) is a single point of
// failure — confirmed live, the one this was pinned to went from "live" to
// permanently "this recording is not available" between sessions. A
// channel's /live URL is far more durable: YouTube resolves it to whatever
// broadcast that channel currently has live, and Lofi Girl's lofi stream
// channel is effectively always live. The related-video tap fallback below
// still exists in case even this somehow isn't live at test time.
const YOUTUBE_URL = "https://www.youtube.com/@LofiGirl/live";

const { log, fail } = makeLogger("audio-test");

/**
 * Finds and taps a real, fully-on-screen related-video row. Confirmed live:
 * uiautomator dump *does* expose this page's accessibility text (the earlier
 * "WebView is opaque to uiautomator" theory was wrong — only the native
 * cookie-consent overlay was opaque), but a lot of rows come back with
 * degenerate zero-height bounds pinned to the bottom edge (e.g.
 * [168,2339,958,2339]) for content that's technically in the accessibility
 * tree but not actually rendered on screen yet — tapping those centers hits
 * nothing. Filtering to rows with real height before tapping.
 */
async function tapRelatedVideo(adbPath, serial) {
  for (let attempt = 0; attempt < 3; attempt++) {
    const rows = await dumpUi(adbPath, serial);
    // A related-video row's text is the combined "<title> by <channel> ...
    // N views|watching" string — the " by " is what distinguishes it from
    // the *primary* (dead) video's own metadata, which renders its view
    // count as a separate, standalone "651,847,164 views" node with no "by"
    // in it. Confirmed live: without requiring " by ", this matched that
    // standalone count instead (near the top of the page) and tapped
    // something that reopened the language/cookie picker rather than any
    // actual video.
    const candidate = rows.find((r) => {
      const [, y1, , y2] = r.bounds;
      return /\bby\s+\S.*\d[\d,]*\s+(views|watching)/i.test(r.text) && y2 - y1 > 20;
    });
    if (candidate) {
      const [x1, y1, x2, y2] = candidate.bounds;
      const cx = Math.round((x1 + x2) / 2);
      const cy = Math.round((y1 + y2) / 2);
      log(`  tapping related video: "${candidate.text.slice(0, 60)}..." @ (${cx},${cy})`);
      adb(adbPath, serial, "shell", "input", "tap", String(cx), String(cy));
      return true;
    }
    adb(adbPath, serial, "shell", "input", "swipe", "540", "2000", "540", "800", "300");
    await sleep(1500);
  }
  return false;
}

async function main() {
  log("waiting for CDP endpoint...");
  const { child, ws, call, adbPath } = await connectToBeo({ skipBuild, cdpPort: CDP_PORT });

  try {
    const serial = await createAndBootAvd({ call, log, device: DEVICE });

    log("opening YouTube in Chrome...");
    adb(adbPath, serial, "shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", YOUTUBE_URL, "com.android.chrome");
    await sleep(6000);

    // Chrome's first-run flow (account picker, sync prompt) and YouTube's
    // cookie dialog only appear on a fresh profile — best-effort dismissal,
    // each step is a no-op if that dialog isn't present.
    log("dismissing Chrome first-run dialogs if present...");
    await tapText(adbPath, serial, /use without an account/i);
    await sleep(1500);
    await tapText(adbPath, serial, /no thanks/i);
    await sleep(1500);

    log("re-opening YouTube (in case first-run consumed the original intent)...");
    adb(adbPath, serial, "shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", YOUTUBE_URL, "com.android.chrome");
    await sleep(6000);

    log("dismissing ad-privacy notice and cookie consent if present...");
    await tapText(adbPath, serial, /^got it$/i);
    await sleep(1500);
    // uiautomator *can* see this page's accessibility text once it's
    // actually rendered (confirmed live via a full dump later in this
    // flow) — the real bug was the scroll gesture itself: the cookie
    // dialog's Accept/Reject buttons are below a long scroll of legal text
    // within the dialog card (roughly the top 60% of the screen), but an
    // earlier version of this swipe started at y=1800, which is *below*
    // the card, in the dimmed background page — it never touched the
    // dialog at all, so "Accept all" was never scrolled into view for
    // tapText to find. Swiping repeatedly from well inside the card's
    // bounds instead.
    for (let i = 0; i < 4; i++) {
      adb(adbPath, serial, "shell", "input", "swipe", "540", "1300", "540", "400", "300");
      await sleep(600);
    }
    await sleep(1000);
    await tapText(adbPath, serial, /^accept all$/i);
    await sleep(3000);

    // Chrome's autoplay policy mutes video that starts playing without a
    // user gesture — confirmed live: the live stream loads and genuinely
    // plays, but silently. The mute toggle isn't reliably text-labeled
    // though — one run showed a "TAP TO UNMUTE" banner (tapText-able),
    // another showed only a small icon-only mute button with zero
    // accessible text (0 dump nodes matched anything), same top-left
    // corner of the video in both cases. A fixed coordinate tap there
    // handles both variants; text-matching alone doesn't.
    log("unmuting video if muted...");
    adb(adbPath, serial, "shell", "input", "tap", "90", "350");
    // Confirmed live: a single fixed sleep here caused a false negative —
    // the unmute tap had actually worked, but hasAudioFocus() was checked
    // before AudioFlinger had registered the new focus, so the script gave
    // up on an already-fixed primary stream and hopped to a (still muted)
    // related video instead. Poll for a few seconds before concluding the
    // tap didn't do anything.
    let unmuted = false;
    for (let i = 0; i < 4 && !unmuted; i++) {
      await sleep(1500);
      unmuted = await hasAudioFocus(adbPath, serial);
    }

    if (!unmuted) {
      log("no video auto-playing — looking for a related video to tap...");
      const tapped = await tapRelatedVideo(adbPath, serial);
      if (tapped) await sleep(5000);
    }

    if (!(await hasAudioFocus(adbPath, serial))) {
      await captureDebugState({ adbPath, serial, log, tag: "audio_test" });
      fail("could not get a video actually playing (no AudioManager focus held) — can't test audio without real playback");
    } else {
      log("  ok — real media playback confirmed (Chrome holds audio focus)");
      await captureAndAnalyzeLogcat({ adbPath, serial, log, fail, playSeconds: 45 });
    }

    await cleanupAvd({ call, child, device: DEVICE, adbPath, serial, log });
    ws.close();
  } catch (e) {
    child.kill("SIGKILL");
    throw e;
  }

  if (process.exitCode === 1) {
    console.error("[audio-test] one or more checks failed — see above");
  } else {
    log("done.");
  }
}

main().catch((e) => {
  console.error("[audio-test] unexpected error:", e);
  process.exitCode = 1;
});
