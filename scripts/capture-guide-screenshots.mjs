#!/usr/bin/env node
// Takes the screenshots the user and administrator guides embed.
//
//   node scripts/capture-guide-screenshots.mjs
//
// The script builds `invenqor-server`, starts it on a private loopback port
// with an empty SQLite state directory, fills it with invented inventory, and
// photographs the console with headless Chrome at 1440x900. Nothing here reads
// a deployment URL from the environment: the Server it talks to is one it
// started itself and deletes on exit, so the run cannot reach, reconfigure or
// erase a real installation.
//
// Output goes to docs/assets/guide/*.png. Re-running overwrites those files.
import {mkdirSync, rmSync, mkdtempSync, readFileSync, existsSync} from "node:fs";
import {spawn, execFileSync} from "node:child_process";
import {dirname, join, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {tmpdir} from "node:os";
import {launchBrowser, connect, navigate, goToHash, screenshot, settle} from "./lib/guide-capture.mjs";
import {seed, randomPassword} from "./lib/guide-seed.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = join(root, "docs", "assets", "guide");
const port = Number(process.env.GUIDE_CAPTURE_PORT || 17931);
const baseURL = `http://127.0.0.1:${port}`;
const INSTANCE_NAME = "invenqor-server-0";
// The console account exists only for the lifetime of the throwaway Server,
// so its password is drawn fresh on every run. Nothing in this repository
// spells out a credential, and the login screen is photographed before the
// password field is filled in.
const admin = {
  username: "demo.admin",
  password: randomPassword(),
  displayName: "데모 관리자",
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

class Client {
  constructor() {
    this.cookies = new Map();
    this.csrf = "";
  }

  header() {
    return [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
  }

  async raw(method, path, body, headers = {}, options = {}) {
    const response = await fetch(`${baseURL}${path}`, {
      method,
      headers: {
        ...(body === undefined ? {} : {"Content-Type": "application/json"}),
        ...(this.cookies.size ? {Cookie: this.header()} : {}),
        ...(this.csrf ? {"X-CSRF-Token": this.csrf} : {}),
        ...headers,
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "manual",
    });
    for (const value of response.headers.getSetCookie?.() ?? []) {
      const [pair] = value.split(";");
      const index = pair.indexOf("=");
      this.cookies.set(pair.slice(0, index).trim(), pair.slice(index + 1).trim());
    }
    if (!response.ok && !options.allowError) {
      throw new Error(`${method} ${path} → HTTP ${response.status}: ${(await response.text()).slice(0, 300)}`);
    }
    return response;
  }

  async json(method, path, body, options = {}) {
    const response = await this.raw(method, path, body, {}, options);
    const text = await response.text();
    return text ? JSON.parse(text) : {};
  }
}

const waitForReady = async () => {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(`${baseURL}/health/ready`);
      if (response.ok) return;
    } catch {
      // The listener is not up yet.
    }
    await sleep(500);
  }
  throw new Error(`Server 가 ${baseURL} 에서 준비되지 않았습니다`);
};

// Types into a controlled React input. Assigning `.value` alone is swallowed
// because React tracks the previous value on the DOM node, and the prototype
// carrying the setter differs between <input> and <textarea>.
const fill = (selector, value) => `(() => {
  const field = document.querySelector(${JSON.stringify(selector)});
  if (!field) throw new Error("입력란을 찾지 못했습니다: " + ${JSON.stringify(selector)});
  const prototype = field.tagName === "TEXTAREA"
    ? window.HTMLTextAreaElement.prototype
    : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value").set
    .call(field, ${JSON.stringify(value)});
  field.dispatchEvent(new Event("input", {bubbles: true}));
  return true;
})()`;

// Brings a panel that sits below the fold to the top of the viewport, so the
// screenshot shows the result table rather than the form that produced it.
const scrollTo = (selector) => `(() => {
  const node = document.querySelector(${JSON.stringify(selector)});
  if (!node) throw new Error("이동할 대상을 찾지 못했습니다: " + ${JSON.stringify(selector)});
  node.scrollIntoView({block: "start"});
  window.scrollBy(0, -24);
  return true;
})()`;

const clickText = (selector, text) => `(() => {
  const node = [...document.querySelectorAll(${JSON.stringify(selector)})]
    .find((candidate) => candidate.textContent.trim().includes(${JSON.stringify(text)}));
  if (!node) throw new Error("누를 대상을 찾지 못했습니다: " + ${JSON.stringify(text)});
  node.click();
  return true;
})()`;

const main = async () => {
  const stateDir = mkdtempSync(join(tmpdir(), "invenqor-guide-state-"));
  const binary = join(stateDir, "invenqor-server");
  process.stdout.write("invenqor-server 빌드 중...\n");
  execFileSync("go", ["build", "-o", binary, "./cmd/invenqor-server"], {
    cwd: join(root, "server"),
    stdio: "inherit",
  });

  const dataDir = join(stateDir, "state");
  mkdirSync(dataDir, {recursive: true});
  // The Server stamps its own `os.Hostname()` into every diagnostic row and
  // request ID, and those are printed on the 감사 로그 and Server 로그 screens.
  // Photographed as-is, the guide would publish the build machine's name, so
  // the process runs in its own UTS namespace under a made-up host name.
  const command = ["-Ur", "--uts", "sh", "-c", `hostname ${INSTANCE_NAME}; exec "$0"`, binary];
  try {
    execFileSync("unshare", ["-Ur", "--uts", "hostname"], {stdio: "ignore"});
  } catch {
    throw new Error(
      "unshare 로 UTS namespace 를 만들 수 없습니다. 그대로 촬영하면 이 장비의 " +
      "호스트 이름이 감사 로그·Server 로그 캡처에 그대로 실립니다.",
    );
  }
  const server = spawn("unshare", command, {
    env: {
      ...process.env,
      INVENQOR_LISTEN_ADDRESS: `127.0.0.1:${port}`,
      INVENQOR_STATE_DIR: dataDir,
      INVENQOR_POSTGRES_DSN: "",
      POSTGRES_DSN: "",
      INVENQOR_AGENT_AUTO_ENROLLMENT: "true",
    },
    stdio: ["ignore", "ignore", "pipe"],
  });
  let browser;
  const discard = (path) => {
    // Chrome keeps writing to its profile for a moment after SIGKILL, so a
    // single rmSync can fail on a directory that is not empty yet.
    for (let attempt = 0; attempt < 20; attempt += 1) {
      try {
        rmSync(path, {recursive: true, force: true, maxRetries: 5, retryDelay: 100});
        return;
      } catch {
        execFileSync("/usr/bin/env", ["sleep", "0.2"]);
      }
    }
  };
  const stop = () => {
    if (browser) {
      browser.chrome.kill("SIGKILL");
      discard(browser.profile);
    }
    server.kill("SIGTERM");
    discard(stateDir);
  };
  process.on("exit", stop);

  try {
    await waitForReady();
    const client = new Client();
    const tokenFile = join(dataDir, "initial-admin.token");
    if (!existsSync(tokenFile)) throw new Error("초기 관리자 token 파일이 없습니다");
    const bootstrapToken = readFileSync(tokenFile, "utf8").trim();
    await client.raw("POST", "/api/v1/bootstrap/admin", {
      username: admin.username,
      password: admin.password,
      display_name: admin.displayName,
    }, {"X-Invenqor-Bootstrap-Token": bootstrapToken});
    process.stdout.write("가짜 자산 채우는 중...\n");
    const login = await client.json("POST", "/api/v1/auth/local/login", {
      username: admin.username,
      password: admin.password,
    });
    client.csrf = login.csrf_token;
    const agentVersion = readFileSync(join(root, "Cargo.toml"), "utf8").match(/^version = "([^"]+)"/m)?.[1];
    if (!agentVersion) throw new Error("Cargo.toml 에서 버전을 읽지 못했습니다");
    const seeded = await seed(client, {agentVersion});
    const host = seeded.assets.find((asset) => asset.name === "web-01.demo.example.com");

    mkdirSync(outputDir, {recursive: true});
    browser = await launchBrowser();
    const session = await connect(browser.endpoint);

    process.stdout.write("화면 촬영 중...\n");
    const shots = [];
    const shoot = async (name) => {
      await screenshot(session, join(outputDir, `${name}.png`));
      shots.push(name);
      process.stdout.write(`  docs/assets/guide/${name}.png\n`);
    };

    // The console is photographed the way an operator reaches it: the login
    // form is filled in and submitted, so the session behind every later screen
    // is a real one rather than an injected cookie.
    await navigate(session, `${baseURL}/`, {
      ready: 'document.querySelector("input[autocomplete=\'username\']")',
    });
    await session.evaluate(fill("input[autocomplete='username']", admin.username));
    await settle(session, {extraWait: 400});
    await shoot("login");

    await session.evaluate(fill("input[autocomplete='current-password']", admin.password));
    await session.evaluate(clickText("button", "안전하게 로그인"));
    await settle(session, {ready: 'document.querySelector("nav")', extraWait: 1500});

    const screens = [
      ["dashboard", "#/dashboard"],
      ["assets", "#/assets"],
      ["software-products", "#/software"],
      ["visualization", "#/visualization"],
      ["agents", "#/agents"],
      ["users", "#/users"],
      ["api-keys", "#/keys"],
      ["audit", "#/audit"],
      ["logs", "#/logs"],
      ["account-security", "#/account"],
      ["preferences", "#/preferences"],
      ["settings-agent-enrollment", "#/settings/agents"],
      ["settings-postgresql", "#/settings/postgresql"],
      ["settings-keycloak", "#/settings/keycloak"],
      ["settings-classification", "#/settings/classification"],
      ["settings-system", "#/settings/system"],
    ];
    for (const [name, hash] of screens) {
      await goToHash(session, hash);
      await shoot(name);
    }

    if (host) {
      await goToHash(session, "#/assets");
      // The row opens the detail drawer from its cells, not from the <tr>.
      await session.evaluate(clickText("tbody tr td", "web-01.demo.example.com"));
      await settle(session, {extraWait: 1200});
      await shoot("asset-detail");
    }

    await goToHash(session, "#/agents");
    await session.evaluate(scrollTo(".agent-cards"));
    await settle(session, {extraWait: 500});
    await shoot("agents-list");

    await goToHash(session, "#/query");
    await session.evaluate(fill("textarea", 'type = "host" and status = "active"'));
    await session.evaluate(clickText("button", "질의 실행"));
    await settle(session, {extraWait: 1500});
    await shoot("query");
    await session.evaluate(scrollTo("section > .panel:last-of-type"));
    await settle(session, {extraWait: 500});
    await shoot("query-result");

    process.stdout.write(`\n${shots.length}장을 docs/assets/guide 에 저장했습니다.\n`);
  } finally {
    stop();
    process.removeListener("exit", stop);
  }
};

main().catch((reason) => {
  process.stderr.write(`화면 캡처 실패: ${reason.message}\n`);
  process.exitCode = 1;
});
