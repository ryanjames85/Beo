// End-to-end test for Beo's self-bundled JDK.
//
// Builds a standalone debug bundle (production frontend baked in, so this
// never touches the vite dev server / port 1420 — safe to run alongside a
// `npm run tauri dev` session) and launches it pointed at a disposable
// BEO_DATA_DIR, isolated from the real per-user install. Drives it over the
// real Chrome DevTools Protocol (WebView2 supports this directly via
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS) to invoke the actual Tauri
// commands a user's click would trigger — not a mock, not a unit test of
// the extraction logic in isolation.
//
// Windows-only for now (WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS is a WebView2
// mechanism); the underlying pure-logic pieces (jdk_extracted_dir_at,
// jdk_home_dir_at, java_bin_at) have OS-agnostic unit tests in lib.rs.
//
// Usage: node scripts/e2e-jdk-bootstrap.mjs [--skip-build]

import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const skipBuild = process.argv.includes("--skip-build");
const repoRoot = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const exePath = join(repoRoot, "src-tauri", "target", "debug", "beo.exe");
const CDP_PORT = 9333;

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

function invoke(ws, pending, idRef, command) {
  return new Promise((resolve) => {
    const id = idRef.next++;
    pending.set(id, resolve);
    ws.send(
      JSON.stringify({
        id,
        method: "Runtime.evaluate",
        params: {
          expression: `window.__TAURI_INTERNALS__.invoke('${command}').then(r => JSON.stringify({ok:true,value:r})).catch(e => JSON.stringify({ok:false,error:String(e)}))`,
          awaitPromise: true,
          returnByValue: true,
        },
      })
    );
  });
}

async function main() {
  if (!skipBuild) {
    log("building standalone debug bundle (bundled frontend, no vite dependency)...");
    execFileSync("npm", ["run", "tauri", "build", "--", "--debug"], {
      cwd: repoRoot,
      stdio: "inherit",
      shell: true,
    });
  }

  if (!existsSync(exePath)) {
    throw new Error(`built binary not found at ${exePath}`);
  }

  const dataDir = mkdtempSync(join(tmpdir(), "beo-e2e-"));
  log(`isolated BEO_DATA_DIR: ${dataDir}`);

  const child = spawn(exePath, [], {
    env: {
      ...process.env,
      BEO_DATA_DIR: dataDir,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
    },
    stdio: "ignore",
  });

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
    log("checking check_java() before install (expect: unavailable)...");
    const before = JSON.parse(await invoke(ws, pending, idRef, "check_java"));
    if (!before.ok) throw new Error(`check_java call failed: ${before.error}`);
    const javaBefore = before.value;
    if (javaBefore.available) {
      fail(`expected no JDK before install, but check_java reported available: ${javaBefore.detail}`);
    } else {
      log(`  ok — "${javaBefore.detail}"`);
    }

    log("invoking install_sdk() — this downloads the real SDK + JDK (~230MB)...");
    const installResult = JSON.parse(await invoke(ws, pending, idRef, "install_sdk"));
    if (!installResult.ok) {
      fail(`install_sdk failed: ${installResult.error}`);
    } else {
      log(`  ok — ${installResult.value}`);
    }

    log("checking check_java() after install (expect: available, Beo's own JDK)...");
    const after = JSON.parse(await invoke(ws, pending, idRef, "check_java"));
    const javaAfter = after.value;
    if (!javaAfter.available) {
      fail(`expected a JDK after install, but check_java still reports unavailable: ${javaAfter.detail}`);
    } else {
      log(`  ok — "${javaAfter.detail}"`);
    }

    const expectedJdkRoot = join(dataDir, "jdk");
    if (!existsSync(expectedJdkRoot)) {
      fail(`expected a JDK to be extracted under isolated data dir at ${expectedJdkRoot}, but it doesn't exist`);
    } else {
      log(`  confirmed on disk under isolated dir: ${expectedJdkRoot}`);
    }

    ws.close();
  } finally {
    child.kill("SIGKILL");
    await sleep(500);
    try {
      rmSync(dataDir, { recursive: true, force: true });
    } catch {
      // best-effort cleanup
    }
  }

  if (process.exitCode === 1) {
    console.error("[e2e] one or more assertions failed");
  } else {
    log("all assertions passed — Beo's JDK bootstrap works end-to-end, fully isolated.");
  }
}

main().catch((e) => {
  console.error("[e2e] unexpected error:", e);
  process.exitCode = 1;
});
