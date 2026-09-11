// Drives headless Chrome over the DevTools protocol and writes the guide
// screenshots.
//
// This module never chooses what to connect to. `capture-guide-screenshots.mjs`
// starts a throwaway Server on a private port with an empty state directory and
// passes that URL in, so a mistyped environment variable cannot point the
// capture run at a real deployment and change its settings.
import {mkdirSync, readFileSync, writeFileSync, existsSync, rmSync} from "node:fs";
import {spawn, execFileSync} from "node:child_process";
import {join} from "node:path";
import {tmpdir} from "node:os";

const VIEWPORT = {width: 1440, height: 900};
const SCALE = 2;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const browserCommand = () => {
  if (process.env.BROWSER) return process.env.BROWSER;
  for (const candidate of [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
  ]) {
    const found = resolveOnPath(candidate);
    if (found) return found;
  }
  throw new Error("Chromium 또는 Google Chrome 이 필요합니다 (BROWSER 로 지정할 수 있습니다)");
};

const resolveOnPath = (name) => {
  try {
    return execFileSync("/usr/bin/env", ["which", name], {encoding: "utf8"}).trim();
  } catch {
    return "";
  }
};

export const launchBrowser = async () => {
  const profile = join(tmpdir(), `invenqor-guide-chrome-${process.pid}`);
  rmSync(profile, {recursive: true, force: true});
  mkdirSync(profile, {recursive: true});
  const chrome = spawn(browserCommand(), [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--hide-scrollbars",
    "--force-device-scale-factor=1",
    "--force-color-profile=srgb",
    "--disable-lcd-text",
    `--window-size=${VIEWPORT.width},${VIEWPORT.height}`,
    `--user-data-dir=${profile}`,
    "--remote-debugging-port=0",
    "about:blank",
  ], {stdio: ["ignore", "ignore", "pipe"]});
  const portFile = join(profile, "DevToolsActivePort");
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (existsSync(portFile)) {
      const port = readFileSync(portFile, "utf8").split("\n")[0].trim();
      if (port) {
        return {chrome, profile, endpoint: `http://127.0.0.1:${port}`};
      }
    }
    await sleep(100);
  }
  chrome.kill("SIGKILL");
  throw new Error("headless Chrome 이 DevTools 포트를 열지 못했습니다");
};

class Session {
  constructor(socket) {
    this.socket = socket;
    this.nextID = 1;
    this.pending = new Map();
    this.sessionID = undefined;
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      const waiting = this.pending.get(message.id);
      if (!waiting) return;
      this.pending.delete(message.id);
      if (message.error) waiting.reject(new Error(`${message.error.message}`));
      else waiting.resolve(message.result);
    });
  }

  send(method, params = {}, useSession = true) {
    const id = this.nextID++;
    const payload = {id, method, params};
    if (useSession && this.sessionID) payload.sessionId = this.sessionID;
    this.socket.send(JSON.stringify(payload));
    return new Promise((resolve, reject) => {
      this.pending.set(id, {resolve, reject});
      setTimeout(() => {
        if (this.pending.delete(id)) reject(new Error(`${method} 응답이 없습니다`));
      }, 30000);
    });
  }

  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.text || "페이지 스크립트 실행 실패");
    }
    return result.result?.value;
  }
}

export const connect = async (endpoint) => {
  const version = await (await fetch(`${endpoint}/json/version`)).json();
  const socket = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, {once: true});
    socket.addEventListener("error", () => reject(new Error("DevTools 연결 실패")), {once: true});
  });
  const session = new Session(socket);
  const target = await session.send("Target.createTarget", {url: "about:blank"}, false);
  const attached = await session.send("Target.attachToTarget", {
    targetId: target.targetId,
    flatten: true,
  }, false);
  session.sessionID = attached.sessionId;
  await session.send("Page.enable");
  await session.send("Runtime.enable");
  await session.send("Network.enable");
  await session.send("Emulation.setDeviceMetricsOverride", {
    ...VIEWPORT,
    deviceScaleFactor: SCALE,
    mobile: false,
  });
  return session;
};

// Waits until the console has stopped fetching. Every screen loads its data
// after the first paint, so screenshotting on `load` freezes a spinner.
export const settle = async (session, {ready, extraWait = 900} = {}) => {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const done = await session.evaluate(`(() => {
      if (document.readyState !== "complete") return false;
      ${ready ? `if (!(${ready})) return false;` : ""}
      return !document.querySelector(".spinner, [aria-busy='true']");
    })()`);
    if (done) break;
    await sleep(250);
  }
  await sleep(extraWait);
};

export const navigate = async (session, url, options) => {
  await session.send("Page.navigate", {url});
  await settle(session, options);
};

// The console is a single page application, so moving between screens is a
// hash change rather than a document load.
export const goToHash = async (session, hash, options) => {
  await session.evaluate(`(() => {
    window.location.hash = ${JSON.stringify(hash)};
    window.scrollTo(0, 0);
    return true;
  })()`);
  await settle(session, options);
};

export const screenshot = async (session, file) => {
  const shot = await session.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: false,
  });
  writeFileSync(file, Buffer.from(shot.data, "base64"));
  return file;
};
