// End-to-end test for the AVD lifecycle + APK sideloading, driven through
// the real compiled app over Chrome DevTools Protocol — not a mock, not a
// unit test of the parsing logic in isolation (that's already covered by
// the fixture tests in avd/lifecycle.rs and friends).
//
// Deliberately local-only, not wired into CI (same call as
// e2e-jdk-bootstrap.mjs): this needs a real, already-installed SDK plus a
// real emulator boot, which takes 1-2 minutes even with hardware
// acceleration and would be far slower or outright unavailable on a
// typical CI runner (no KVM/HAXM/WHPX access). Run it locally before a
// release, not on every push.
//
// Unlike e2e-jdk-bootstrap.mjs, this does NOT isolate BEO_DATA_DIR — it
// runs against whatever SDK is already installed on this machine (the
// same real environment every manual CDP verification in this project has
// used), so repeat runs don't re-download a ~1GB system image every time.
// It creates one clearly-named throwaway device and deletes it afterward.
//
// Requires: an already-installed SDK (run the app once / the JDK e2e test
// first if this is a fresh machine) and Windows (WebView2 remote
// debugging + the `.exe` paths below).
//
// Usage: node scripts/e2e-avd-sideload.mjs [--skip-build]

import { execFileSync, spawn } from "node:child_process";
import { existsSync, rmSync, unlinkSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const skipBuild = process.argv.includes("--skip-build");
const repoRoot = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const exePath = join(repoRoot, "src-tauri", "target", "debug", "beo.exe");
const CDP_PORT = 9334;
const DEVICE = "e2e_sideload_test";

function log(msg) {
  console.log(`[e2e] ${msg}`);
}

function fail(msg) {
  console.error(`[e2e] FAIL: ${msg}`);
  process.exitCode = 1;
}

async function waitForCdp(port, timeoutMs) {
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

function adbDevices(adbPath) {
  const out = execFileSync(adbPath, ["devices"], { encoding: "utf8" });
  return out
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.startsWith("emulator-"))
    .map((l) => l.split(/\s+/)[0]);
}

async function main() {
  if (!skipBuild) {
    log("building standalone debug bundle...");
    execFileSync("npm", ["run", "tauri", "build", "--", "--debug"], {
      cwd: repoRoot,
      stdio: "inherit",
      shell: true,
    });
  }
  if (!existsSync(exePath)) throw new Error(`built binary not found at ${exePath}`);

  const child = spawn(exePath, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}` },
    stdio: "ignore",
  });

  let apkPath;
  try {
    log("waiting for CDP endpoint...");
    const pages = await waitForCdp(CDP_PORT, 20000);
    const page = pages.find((p) => p.type === "page");
    if (!page) throw new Error("no page target exposed over CDP");

    const ws = new WebSocket(page.webSocketDebuggerUrl);
    const pending = new Map();
    const idRef = { next: 1 };
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data);
      if (pending.has(msg.id)) {
        pending.get(msg.id)(msg.result?.result?.value);
        pending.delete(msg.id);
      }
    });
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve);
      ws.addEventListener("error", reject);
    });

    const call = (cmd, args) => invoke(ws, pending, idRef, cmd, args).then((r) => JSON.parse(r));

    log("checking sdk_status() — this test needs an already-installed SDK...");
    const sdkOk = await call("sdk_status");
    if (!sdkOk.ok || !sdkOk.value) {
      throw new Error("no SDK installed on this machine — run the app (or the JDK e2e test) once first");
    }
    log("  ok — SDK present");

    const sdkPath = (await call("sdk_path")).value;
    const adbPath = join(sdkPath, "platform-tools", "adb.exe");
    if (!existsSync(adbPath)) throw new Error(`adb not found at ${adbPath}`);

    log("picking a real Play Store x86_64 image...");
    const images = (await call("list_available_images")).value;
    const image = images.find((i) => i.includes("google_apis_playstore") && i.includes("x86_64"));
    if (!image) throw new Error("no google_apis_playstore x86_64 image available");
    log(`  using ${image}`);

    log(`create_avd("${DEVICE}")...`);
    const created = await call("create_avd", { name: DEVICE, imageId: image, device: "pixel_6" });
    if (!created.ok) throw new Error(`create_avd failed: ${created.error}`);
    log(`  ok — ${created.value}`);

    log("launch_avd (headless)...");
    const launched = await call("launch_avd", { name: DEVICE, headless: true, shareClipboard: true });
    if (!launched.ok) throw new Error(`launch_avd failed: ${launched.error}`);
    log(`  ok — ${launched.value}`);

    log("waiting for the device to fully boot (polling rotate_avd until it succeeds)...");
    let booted = false;
    for (let i = 0; i < 30 && !booted; i++) {
      await sleep(5000);
      const running = await call("list_running_avds");
      if (!running.ok || !running.value.includes(DEVICE)) continue;
      const rot = await call("rotate_avd", { name: DEVICE, orientation: "landscape" });
      if (rot.ok) {
        booted = true;
        await call("rotate_avd", { name: DEVICE, orientation: "portrait" });
      }
    }
    if (!booted) {
      fail("device never finished booting within the timeout");
    } else {
      log("  ok — window manager responsive (rotate_avd round-trip succeeded)");
    }

    if (booted) {
      const serials = adbDevices(adbPath);
      if (serials.length === 0) throw new Error("adb reports no running emulator, but Beo thinks one is up");
      const serial = serials[0];

      // Real finding from an earlier run of this exact script: a
      // successful `rotate_avd` (window manager responding) is NOT
      // sufficient proof the device is ready for `pm install` — the first
      // run here hit "device is still booting" from adb itself even
      // though rotate had just succeeded, meaning PackageManagerService
      // can still be initializing after window manager already responds.
      // Poll `sys.boot_completed` directly for 3 consecutive reads, the
      // same threshold `launch_avd`'s own internal boot-completion check
      // uses, before trusting the device is actually ready.
      log("confirming full boot via sys.boot_completed (3 consecutive reads)...");
      let consecutiveOk = 0;
      for (let i = 0; i < 20 && consecutiveOk < 3; i++) {
        // A transient `adb shell` hiccup (exit 255, empty output) right
        // after boot/reconnect is normal adb behavior, not a real failure
        // — treat it the same as "not ready yet" rather than crashing the
        // whole script over one flaky poll.
        let prop = "";
        try {
          prop = execFileSync(adbPath, ["-s", serial, "shell", "getprop", "sys.boot_completed"], {
            encoding: "utf8",
          }).trim();
        } catch {
          // fall through with prop === "", counted as not-yet-booted below
        }
        consecutiveOk = prop === "1" ? consecutiveOk + 1 : 0;
        if (consecutiveOk < 3) await sleep(3000);
      }
      if (consecutiveOk < 3) {
        fail("sys.boot_completed never held steady at 1 — device may still be initializing");
      } else {
        log("  ok — sys.boot_completed held at 1 for 3 consecutive reads");
      }

      log("pulling a real installed APK off the device to use as sideload payload...");
      const remotePath = execFileSync(adbPath, ["-s", serial, "shell", "pm", "path", "com.google.android.deskclock"], {
        encoding: "utf8",
      })
        .trim()
        .replace(/^package:/, "");
      apkPath = join(repoRoot, "e2e-sideload-payload.apk");
      execFileSync(adbPath, ["-s", serial, "pull", remotePath, apkPath]);
      if (!existsSync(apkPath)) throw new Error("adb pull did not produce a local file");
      log(`  ok — pulled ${remotePath}`);

      log("install_apk() onto the real, named device...");
      const installed = await call("install_apk", { name: DEVICE, apkPath });
      if (!installed.ok || !String(installed.value).toLowerCase().includes("success")) {
        fail(`install_apk did not report success: ${JSON.stringify(installed)}`);
      } else {
        log(`  ok — ${JSON.stringify(installed.value).slice(0, 80)}`);
      }

      log("install_apk() against a not-running device name (expect: clean failure, not silent adb-default fallback)...");
      const badInstall = await call("install_apk", { name: "not_a_real_device_e2e", apkPath });
      if (badInstall.ok || !String(badInstall.error).includes("Couldn't find a running emulator")) {
        fail(`expected a clean "couldn't find a running emulator" error, got: ${JSON.stringify(badInstall)}`);
      } else {
        log("  ok — correctly rejected");
      }
    }

    log("stopping and deleting the throwaway device...");
    await call("stop_avd", { name: DEVICE });
    await sleep(2000);
    const deleted = await call("delete_avd", { name: DEVICE });
    if (!deleted.ok) log(`  (non-fatal) delete_avd reported: ${deleted.error}`);

    ws.close();
  } finally {
    child.kill("SIGKILL");
    await sleep(500);
    if (apkPath && existsSync(apkPath)) {
      try {
        unlinkSync(apkPath);
      } catch {
        // best-effort cleanup
      }
    }
    // avdmanager's own delete doesn't always remove a fully-booted device's
    // directory cleanly (open file handles from the just-killed process) —
    // same thing observed manually earlier this session. Best-effort sweep.
    try {
      const avdDir = join(homedir(), ".android", "avd", `${DEVICE}.avd`);
      const avdIni = join(homedir(), ".android", "avd", `${DEVICE}.ini`);
      if (existsSync(avdDir)) rmSync(avdDir, { recursive: true, force: true });
      if (existsSync(avdIni)) rmSync(avdIni, { force: true });
    } catch {
      // best-effort cleanup
    }
  }

  if (process.exitCode === 1) {
    console.error("[e2e] one or more assertions failed");
  } else {
    log("all assertions passed — AVD lifecycle + sideloading work end-to-end.");
  }
}

main().catch((e) => {
  console.error("[e2e] unexpected error:", e);
  process.exitCode = 1;
});
