// Shared plumbing for Beo's audio-glitch test suite (test-audio-youtube.mjs,
// test-audio-file.mjs): building/launching the real compiled binary,
// driving it over CDP, creating/booting a throwaway AVD through Beo's real
// commands, adb helpers, and logcat glitch analysis. Split out once a
// second test script needed the exact same scaffolding — see TODO.md's
// "Tenth batch" for the history of bugs this scaffolding already fixed
// once; don't re-diagnose them if they resurface here.

import { execFileSync, spawn } from "node:child_process";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export const repoRoot = new URL("../..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");

// Beo runs adb/emulator against a private adb server (see
// ANDROID_ADB_SERVER_PORT in src-tauri/src/util.rs's android_tool) so it
// never collides with a system adb install. Every adb invocation from these
// scripts needs the same env var or it talks to a *different* (default port
// 5037) adb server that has never heard of the emulator Beo just launched —
// confirmed live: without this, `adb devices` reported nothing even while
// the emulator was genuinely up.
export const ADB_ENV = { ...process.env, ANDROID_ADB_SERVER_PORT: "5039" };

export function makeLogger(tag) {
  return {
    log(msg) {
      console.log(`[${tag}] ${msg}`);
    },
    fail(msg) {
      console.error(`[${tag}] FAIL: ${msg}`);
      process.exitCode = 1;
    },
  };
}

export async function waitForCdp(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/list`);
      if (res.ok) return await res.json();
    } catch {
      // not up yet
    }
    await sleep(500);
  }
  throw new Error(`CDP endpoint on port ${port} never came up`);
}

function invoke(ws, pending, idRef, command, args) {
  return new Promise((resolve) => {
    const id = idRef.next++;
    pending.set(id, resolve);
    const argsJson = JSON.stringify(args ?? {});
    ws.send(
      JSON.stringify({
        id,
        method: "Runtime.evaluate",
        params: {
          expression: `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${argsJson}).then(r => JSON.stringify({ok:true,value:r})).catch(e => JSON.stringify({ok:false,error:String(e && e.message || e)}))`,
          awaitPromise: true,
          returnByValue: true,
        },
      })
    );
  });
}

export function adb(adbPath, serial, ...args) {
  // 45s of real logcat output (the whole point of this suite) comfortably
  // exceeds Node's default 1MB maxBuffer — confirmed live, `logcat -d`
  // threw ENOBUFS on a genuinely successful run, right after real audio
  // playback was confirmed. 64MB is generous headroom for a single dump.
  return execFileSync(adbPath, ["-s", serial, ...args], {
    encoding: "utf8",
    env: ADB_ENV,
    maxBuffer: 64 * 1024 * 1024,
  });
}

/** Parses a `uiautomator dump` XML into {text, bounds:[x1,y1,x2,y2]} rows. */
function parseDump(xml) {
  const rows = [];
  const nodeRe = /text="([^"]*)"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g;
  let m;
  while ((m = nodeRe.exec(xml))) {
    if (m[1] === "") continue;
    rows.push({ text: m[1], bounds: [Number(m[2]), Number(m[3]), Number(m[4]), Number(m[5])] });
  }
  return rows;
}

export async function dumpUi(adbPath, serial) {
  const local = join(repoRoot, ".audio_test_dump.xml");
  // `uiautomator dump` intermittently fails with "null root node returned
  // by UiTestAutomationBridge" (a known flaky condition, usually mid page
  // transition) — confirmed live: it silently produced zero usable rows
  // once, which made a real tap get skipped without any error surfacing.
  // Retrying a couple of times before giving up beats treating a transient
  // hiccup as "the button just isn't there."
  for (let attempt = 0; attempt < 3; attempt++) {
    const dumpOut = adb(adbPath, serial, "shell", "uiautomator", "dump", "/sdcard/audio_test_dump.xml");
    if (/ERROR|null root node/i.test(dumpOut)) {
      await sleep(1000);
      continue;
    }
    execFileSync(adbPath, ["-s", serial, "pull", "/sdcard/audio_test_dump.xml", local], { env: ADB_ENV });
    const xml = readFileSync(local, "utf8");
    rmSync(local, { force: true });
    const rows = parseDump(xml);
    if (rows.length > 0) return rows;
    await sleep(1000);
  }
  return [];
}

/** Taps the center of the first element whose text matches `pattern`. Returns true if found and tapped. */
export async function tapText(adbPath, serial, pattern) {
  const rows = await dumpUi(adbPath, serial);
  const row = rows.find((r) => pattern.test(r.text));
  if (!row) return false;
  const [x1, y1, x2, y2] = row.bounds;
  const cx = Math.round((x1 + x2) / 2);
  const cy = Math.round((y1 + y2) / 2);
  adb(adbPath, serial, "shell", "input", "tap", String(cx), String(cy));
  return true;
}

export async function hasAudioFocus(adbPath, serial) {
  const out = adb(adbPath, serial, "shell", "dumpsys", "audio");
  // Originally checked for "USAGE_MEDIA" + "gain: GAIN" in the legacy Audio
  // Focus stack section — confirmed live that section can be genuinely
  // empty even during real, active playback (a bare MediaPlayer.start()
  // with no explicit AudioAttributes/AudioFocusRequest never populates it,
  // which the diagnostics app's native audio-file test hit). An
  // AudioPlaybackConfiguration entry with state:started is the one signal
  // both a Chrome-driven video and a plain native MediaPlayer reliably
  // produce, regardless of the app's own AudioFocus-request hygiene.
  return /state:\s*started/i.test(out);
}

/**
 * Builds (unless skipped) and launches the real compiled Beo binary,
 * connects over CDP, and returns a `call(command, args)` helper that drives
 * real Tauri commands exactly as the app's own UI would. Only
 * `npm run tauri build -- --debug` produces the binary this needs — a plain
 * `cargo build`/`cargo check` run elsewhere in the same session silently
 * overwrites `target/debug/beo.exe` with a *dev-mode* build that loads
 * `http://localhost:1420` (the Vite dev server) instead of the bundled
 * frontend, and `window.__TAURI_INTERNALS__` never appears on that page —
 * confirmed live, this cost a full debugging session before the cause was
 * found. Don't trust `--skip-build` if any bare cargo command may have run
 * since the last real build.
 */
export async function connectToBeo({ skipBuild, cdpPort }) {
  const exePath = join(repoRoot, "src-tauri", "target", "debug", "beo.exe");
  if (!skipBuild) {
    execFileSync("npm", ["run", "tauri", "build", "--", "--debug"], {
      cwd: repoRoot,
      stdio: "inherit",
      shell: true,
    });
  }
  if (!existsSync(exePath)) throw new Error(`built binary not found at ${exePath}`);

  const child = spawn(exePath, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}` },
    stdio: "ignore",
  });

  const pages = await waitForCdp(cdpPort, 20000);
  const page = pages.find((p) => p.type === "page");
  if (!page) {
    child.kill("SIGKILL");
    throw new Error("no page target exposed over CDP");
  }

  const ws = new WebSocket(page.webSocketDebuggerUrl);
  const pending = new Map();
  const idRef = { next: 1 };
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(ev.data);
    if (pending.has(msg.id)) {
      if (msg.error) console.error("[beo-audio-test] CDP error:", JSON.stringify(msg.error));
      if (msg.result?.exceptionDetails) {
        console.error("[beo-audio-test] JS exception:", JSON.stringify(msg.result.exceptionDetails));
      }
      pending.get(msg.id)(msg.result?.result?.value);
      pending.delete(msg.id);
    }
  });
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve);
    ws.addEventListener("error", reject);
  });

  const call = (cmd, args) => invoke(ws, pending, idRef, cmd, args).then((r) => JSON.parse(r));

  // The page target can be reachable over CDP slightly before the app's own
  // JS (window.__TAURI_INTERNALS__) has finished initializing — a fixed
  // pause here is cheaper than retry logic for a one-time cost.
  await sleep(2000);

  const sdkOk = await call("sdk_status");
  if (!sdkOk.ok || !sdkOk.value) {
    ws.close();
    child.kill("SIGKILL");
    throw new Error("no SDK installed — run the app once first");
  }
  const sdkPath = (await call("sdk_path")).value;
  const adbPath = join(sdkPath, "platform-tools", "adb.exe");

  return { child, ws, call, adbPath };
}

/**
 * Creates and boots a throwaway AVD via Beo's real `create_avd`/`launch_avd`
 * commands (so it exercises whatever `-gpu`/`-feature` flags are currently
 * pinned in `lifecycle.rs`), and returns its adb serial once actually ready
 * for use. Three separate race conditions are folded in here, each found
 * live: `list_running_avds` can report "booted" before adb has attached the
 * serial at all; adb itself can report a serial before boot has genuinely
 * finished (`sys.boot_completed` polling below); and both checks need
 * Beo's private adb server port or they see nothing.
 */
export async function createAndBootAvd({ call, log, device, deviceProfile = "pixel_6" }) {
  const images = (await call("list_available_images")).value;
  const image = images.find((i) => i.includes("android-36;") && i.includes("google_apis_playstore") && i.includes("x86_64"));
  if (!image) throw new Error("no android-36 google_apis_playstore x86_64 image available");

  log(`create_avd("${device}")...`);
  const created = await call("create_avd", { name: device, imageId: image, device: deviceProfile });
  if (!created.ok) throw new Error(`create_avd failed: ${created.error}`);

  log("launch_avd (real Beo defaults — whatever -gpu/-feature flags launch_avd currently uses)...");
  const launched = await call("launch_avd", { name: device, headless: false, shareClipboard: true });
  if (!launched.ok) throw new Error(`launch_avd failed: ${launched.error}`);

  log("waiting for full boot...");
  let booted = false;
  for (let i = 0; i < 30 && !booted; i++) {
    await sleep(5000);
    const running = await call("list_running_avds");
    if (running.ok && running.value.includes(device)) booted = true;
  }
  if (!booted) throw new Error("device never finished booting");
  log("  ok — booted");

  const sdkPath = (await call("sdk_path")).value;
  const adbPath = join(sdkPath, "platform-tools", "adb.exe");

  let serial;
  for (let i = 0; i < 20 && !serial; i++) {
    const devicesOut = execFileSync(adbPath, ["devices"], { encoding: "utf8", env: ADB_ENV });
    serial = devicesOut
      .split("\n")
      .map((l) => l.trim())
      .find((l) => l.startsWith("emulator-") && l.endsWith("device"))
      ?.split(/\s+/)[0];
    if (!serial) await sleep(3000);
  }
  if (!serial) throw new Error("adb reports no running emulator");

  log("confirming full boot via sys.boot_completed (3 consecutive reads)...");
  let consecutiveOk = 0;
  for (let i = 0; i < 20 && consecutiveOk < 3; i++) {
    let prop = "";
    try {
      prop = execFileSync(adbPath, ["-s", serial, "shell", "getprop", "sys.boot_completed"], {
        encoding: "utf8",
        env: ADB_ENV,
      }).trim();
    } catch {
      // transient adb hiccup right after attach — treat as not-ready-yet
    }
    consecutiveOk = prop === "1" ? consecutiveOk + 1 : 0;
    if (consecutiveOk < 3) await sleep(3000);
  }
  if (consecutiveOk < 3) throw new Error("sys.boot_completed never held steady at 1");
  log("  ok — sys.boot_completed held at 1 for 3 consecutive reads");

  return serial;
}

/** Clears logcat, waits for the given duration of real playback, then reports any glitch signatures found. Returns true if playback looked clean (no explicit underrun/glitch lines). */
export async function captureAndAnalyzeLogcat({ adbPath, serial, log, fail, playSeconds = 45 }) {
  log(`clearing logcat and letting it play for ${playSeconds}s...`);
  adb(adbPath, serial, "logcat", "-c");
  await sleep(playSeconds * 1000);

  const logs = adb(adbPath, serial, "logcat", "-d");
  const glitchLines = logs.split("\n").filter((l) => /underrun|xrun|glitch|audio.*drop/i.test(l));
  const audioFlingerLines = logs.split("\n").filter((l) => /AudioFlinger|AudioTrack/i.test(l));

  log(`AudioFlinger/AudioTrack log lines during playback (${audioFlingerLines.length}):`);
  audioFlingerLines.forEach((l) => console.log(`    ${l.trim()}`));

  let clean = true;
  if (glitchLines.length > 0) {
    clean = false;
    fail(`found ${glitchLines.length} explicit underrun/glitch log line(s) — real evidence of audio trouble:`);
    glitchLines.forEach((l) => console.log(`    ${l.trim()}`));
  } else {
    log("  no explicit underrun/xrun/glitch log lines found during playback");
  }

  const mismatch = audioFlingerLines.find((l) => /mismatch between requested flags/i.test(l));
  if (mismatch) {
    log("  NOTE: found an AudioFlinger buffer-strategy mismatch (deep-buffer request falling back to");
    log("        the primary/short-buffer output path) — not proof of an audible glitch, but a real,");
    log("        objective reason this path is more underrun-prone than it should be:");
    log(`        ${mismatch.trim()}`);
  }

  log("");
  log("This script can tell you whether the log shows explicit underruns — it");
  log("cannot confirm whether what's playing actually sounds clean. Listen for");
  log("yourself too before concluding the audio issue is resolved.");

  return clean;
}

export async function captureDebugState({ adbPath, serial, log, tag }) {
  const shotLocal = join(repoRoot, `.${tag}_debug.png`);
  try {
    adb(adbPath, serial, "shell", "screencap", "-p", `/sdcard/${tag}_debug.png`);
    execFileSync(adbPath, ["-s", serial, "pull", `/sdcard/${tag}_debug.png`, shotLocal], { env: ADB_ENV });
    log(`  debug screenshot saved to ${shotLocal}`);
  } catch (e) {
    log(`  (could not capture debug screenshot: ${e.message})`);
  }
  try {
    const rows = await dumpUi(adbPath, serial);
    log(`  ui dump (${rows.length} text nodes):`);
    rows.forEach((r) => console.log(`    "${r.text}" @ [${r.bounds.join(",")}]`));
  } catch (e) {
    log(`  (could not dump ui: ${e.message})`);
  }
}

/**
 * Stops and deletes the throwaway device, verifying it's actually gone
 * before touching its files. Confirmed live, repeatedly, across this
 * session: deleting an AVD's directory (or even just moving past it) while
 * its qemu process is still alive leaves that process holding file locks
 * on snapshots/images, which then breaks the *next* build or boot with
 * confusing "access denied"/"device offline" errors that have nothing to
 * do with whatever's actually being tested next. `stop_avd` (or the CDP
 * connection carrying it) can fail transiently without the emulator
 * actually being dead — polling `adb devices` for the serial to vanish
 * catches that instead of assuming success and deleting anyway.
 */
export async function cleanupAvd({ call, child, device, adbPath, serial, log }) {
  log("stopping and deleting the throwaway device...");
  try {
    await call("stop_avd", { name: device });
  } catch {
    // fall through to the poll below — it decides whether this actually
    // worked, rather than trusting this call's success/failure alone
  }

  let stopped = false;
  if (adbPath && serial) {
    for (let i = 0; i < 10 && !stopped; i++) {
      await sleep(1000);
      try {
        const out = execFileSync(adbPath, ["devices"], { encoding: "utf8", env: ADB_ENV });
        stopped = !out.includes(serial);
      } catch {
        // adb hiccup mid-poll — try again rather than treating it as stopped
      }
    }
    if (!stopped) {
      log(`  device didn't stop gracefully — force-killing serial ${serial} directly`);
      try {
        execFileSync(adbPath, ["-s", serial, "emu", "kill"], { env: ADB_ENV });
        await sleep(2000);
      } catch {
        // best-effort — the file-lock risk below is the real concern, not this
      }
    }
  } else {
    // No adbPath/serial available to poll with (older call sites) — fall
    // back to the original fixed pause rather than skipping the wait.
    await sleep(2000);
  }

  try {
    await call("delete_avd", { name: device });
  } catch {
    // best-effort — the direct filesystem cleanup below is the real cleanup
  }
  child.kill("SIGKILL");
  await sleep(500);
  try {
    const avdDir = join(homedir(), ".android", "avd", `${device}.avd`);
    const avdIni = join(homedir(), ".android", "avd", `${device}.ini`);
    if (existsSync(avdDir)) rmSync(avdDir, { recursive: true, force: true });
    if (existsSync(avdIni)) rmSync(avdIni, { force: true });
  } catch (e) {
    log(`  (couldn't fully remove leftover AVD files: ${e.message} — likely still locked; check for a stray qemu process)`);
  }
}
