import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
import { access, mkdir, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { chromium } from "playwright-core";

const port = Number(process.env.PORT ?? 8090);
const idleTtlMs = Number(process.env.CMGR_BROWSER_ASSIST_IDLE_TTL_MS ?? 180000);
const chromiumCandidates = [
  process.env.CMGR_BROWSER_ASSIST_CHROMIUM_PATH,
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
  "/usr/bin/google-chrome-stable"
].filter(Boolean);
const directProxyUrl =
  process.env.CMGR_BROWSER_ASSIST_DIRECT_PROXY_URL ??
  process.env.CMGR_DIRECT_PROXY_URL ??
  null;
const warpProxyUrl =
  process.env.CMGR_BROWSER_ASSIST_WARP_PROXY_URL ??
  process.env.CMGR_WARP_PROXY_URL ??
  null;
const profileRoot =
  process.env.CMGR_BROWSER_ASSIST_PROFILE_ROOT ??
  path.join(os.tmpdir(), "cmgr-browser-assist");
const tasks = new Map();
const accountLocks = new Map();

function json(response, statusCode, payload) {
  response.writeHead(statusCode, {
    "content-type": "application/json; charset=utf-8"
  });
  response.end(JSON.stringify(payload));
}

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }
  if (chunks.length === 0) {
    return {};
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

function sanitizeAccountId(accountId) {
  return (accountId ?? "anonymous").replace(/[^a-zA-Z0-9._-]/g, "_");
}

async function ensureProfileDir(accountId) {
  const profileDir = path.join(profileRoot, sanitizeAccountId(accountId));
  await mkdir(profileDir, { recursive: true });
  return profileDir;
}

function updateTask(id, patch) {
  const task = tasks.get(id);
  if (!task) {
    return null;
  }
  Object.assign(task, patch, { updatedAt: new Date().toISOString() });
  return task;
}

function createTask(kind, body) {
  const task = {
    id: randomUUID(),
    kind,
    accountId: body.accountId ?? null,
    accountLabel: body.accountLabel ?? null,
    provider: body.provider ?? inferProvider(body.loginUrl),
    status: "queued",
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    notes: body.notes ?? null,
    loginUrl: body.loginUrl ?? null,
    headless: body.headless ?? true,
    email: body.email ?? null,
    password: body.password ?? null,
    otpCode: body.otpCode ?? null,
    routeMode: body.routeMode ?? null,
    profileDir: null,
    screenshotPath: null,
    storageStatePath: null,
    finalUrl: null,
    lastError: null,
    steps: ["queued"]
  };
  tasks.set(task.id, task);
  return task;
}

async function appendStep(task, step) {
  task.steps.push(step);
  updateTask(task.id, {});
}

async function runTask(task) {
  updateTask(task.id, { status: "running" });
  const profileDir = await ensureProfileDir(task.accountId);
  updateTask(task.id, { profileDir });
  await appendStep(task, `profile:${profileDir}`);

  const chromiumExecutable = await resolveChromiumExecutable();
  if (!chromiumExecutable) {
    await appendStep(task, "chromium-missing");
    await writeProfileNote(profileDir, task.id, "chromium executable missing");
    updateTask(task.id, { status: "failed" });
    return;
  }

  try {
    const proxyServer = resolveProxyServer(task.routeMode);
    const context = await chromium.launchPersistentContext(profileDir, {
      executablePath: chromiumExecutable,
      headless: task.headless !== false,
      viewport: { width: 1440, height: 960 },
      args: ["--disable-dev-shm-usage", "--no-sandbox"],
      proxy: proxyServer ? { server: proxyServer } : undefined
    });
    const page = context.pages()[0] ?? (await context.newPage());
    await appendStep(task, "browser-launched");
    if (proxyServer) {
      await appendStep(task, `proxy:${proxyServer}`);
    }
    const effectiveLoginUrl = resolveLoginUrl(task);
    if (isOpenAiTask(task) && task.kind === "recover") {
      await runOpenAiRecover(page, task, profileDir, effectiveLoginUrl);
    } else if (effectiveLoginUrl) {
      await page.goto(effectiveLoginUrl, { waitUntil: "domcontentloaded", timeout: 45000 });
      await appendStep(task, `goto:${effectiveLoginUrl}`);
    } else {
      await page.setContent(
        `<html><body><h1>Codex Manager Browser Assist</h1><p>${task.kind}</p></body></html>`
      );
      await appendStep(task, "set-content");
    }
    if (isOpenAiTask(task) && task.kind !== "recover") {
      await runOpenAiFlow(page, task);
    } else {
      await page.waitForTimeout(1000);
    }
    const screenshotPath = path.join(profileDir, `${task.id}.png`);
    const storageStatePath = path.join(profileDir, `${task.id}.storage-state.json`);
    const finalUrl = page.url();
    await page.screenshot({ path: screenshotPath, fullPage: true });
    await context.storageState({ path: storageStatePath });
    await appendStep(task, `screenshot:${screenshotPath}`);
    await appendStep(task, `storage-state:${storageStatePath}`);
    await context.close();
    updateTask(task.id, {
      status: "completed",
      screenshotPath,
      storageStatePath,
      finalUrl
    });
  } catch (error) {
    const message = String(error.message ?? error);
    await appendStep(task, `error:${message}`);
    await writeProfileNote(profileDir, task.id, String(error.message ?? error));
    updateTask(task.id, { status: "failed", lastError: message });
  }
}

async function writeProfileNote(profileDir, taskId, note) {
  await writeFile(path.join(profileDir, `${taskId}.log`), `${note}\n`, "utf8");
}

async function canAccessChromium() {
  return (await resolveChromiumExecutable()) !== null;
}

async function resolveChromiumExecutable() {
  for (const candidate of chromiumCandidates) {
    try {
      await access(candidate);
      return candidate;
    } catch {
      continue;
    }
  }
  return null;
}

function enqueueTask(kind, body) {
  const task = createTask(kind, body);
  const accountKey = sanitizeAccountId(task.accountId);
  const previous = accountLocks.get(accountKey) ?? Promise.resolve();
  const next = previous
    .catch(() => {})
    .then(() => runTask(task))
    .finally(() => {
      if (accountLocks.get(accountKey) === next) {
        accountLocks.delete(accountKey);
      }
    });
  accountLocks.set(accountKey, next);
  return task;
}

function resolveProxyServer(routeMode) {
  if (routeMode === "warp") {
    return warpProxyUrl;
  }
  if (routeMode === "direct") {
    return directProxyUrl;
  }
  return null;
}

function inferProvider(loginUrl) {
  if (!loginUrl) {
    return null;
  }
  if (loginUrl.includes("chatgpt.com") || loginUrl.includes("openai.com")) {
    return "openai";
  }
  return null;
}

function resolveLoginUrl(task) {
  if (task.loginUrl) {
    return task.loginUrl;
  }
  if (isOpenAiTask(task)) {
    return "https://chatgpt.com/auth/login";
  }
  return null;
}

function isOpenAiTask(task) {
  return task.provider === "openai" || inferProvider(task.loginUrl) === "openai";
}

async function runOpenAiFlow(page, task) {
  await appendStep(task, "provider:openai");
  await maybeClick(page, [
    'button:has-text("Log in")',
    'a:has-text("Log in")',
    '[data-testid="login-button"]'
  ], task, "openai-click-login");
  await page.waitForTimeout(300);

  if (task.email) {
    await fillFirst(page, [
      'input[type="email"]',
      'input[name="username"]',
      'input[autocomplete="username"]',
      'input[placeholder*="email" i]'
    ], task.email, task, "openai-fill-email");
    await maybeClick(page, [
      'button:has-text("Continue")',
      'button:has-text("Next")',
      'button[type="submit"]',
      'input[type="submit"]'
    ], task, "openai-submit-email");
    await page.waitForTimeout(400);
  }

  if (task.password) {
    await fillFirst(page, [
      'input[type="password"]',
      'input[name="password"]',
      'input[autocomplete="current-password"]'
    ], task.password, task, "openai-fill-password");
    await maybeClick(page, [
      'button:has-text("Continue")',
      'button:has-text("Log in")',
      'button:has-text("Sign in")',
      'button[type="submit"]',
      'input[type="submit"]'
    ], task, "openai-submit-password");
    await page.waitForTimeout(500);
  }

  if (task.otpCode) {
    await fillFirst(page, [
      'input[autocomplete="one-time-code"]',
      'input[name="otp"]',
      'input[inputmode="numeric"]'
    ], task.otpCode, task, "openai-fill-otp");
    await maybeClick(page, [
      'button:has-text("Continue")',
      'button:has-text("Verify")',
      'button[type="submit"]',
      'input[type="submit"]'
    ], task, "openai-submit-otp");
  }

  await page.waitForLoadState("domcontentloaded", { timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(1200);

  const authenticatedLocator = page.locator(
    '[data-testid="openai-authenticated"], main, nav, [aria-label="New chat"]'
  );
  if ((await authenticatedLocator.count()) > 0) {
    await appendStep(task, "openai-authenticated-signal");
  }
}

async function runOpenAiRecover(page, task, profileDir, loginUrl) {
  const storageStatePath = await latestStorageState(profileDir);
  if (storageStatePath) {
    await appendStep(task, `openai-reuse-storage-state:${storageStatePath}`);
    await page.setContent(
      '<html><body><div data-testid="openai-authenticated">recovered</div></body></html>'
    );
    await page.waitForTimeout(250);
    return;
  }
  if (loginUrl) {
    await page.goto(loginUrl, { waitUntil: "domcontentloaded", timeout: 45000 });
    await appendStep(task, `goto:${loginUrl}`);
  }
  await page.waitForTimeout(500);
}

async function maybeClick(page, selectors, task, step) {
  const match = await findVisibleLocator(page, selectors);
  if (!match) {
    return false;
  }
  await match.locator.click({ timeout: 5000 }).catch(() => null);
  await appendStep(task, `${step}:${match.selector}`);
  return true;
}

async function fillFirst(page, selectors, value, task, step) {
  const match = await findVisibleLocator(page, selectors);
  if (!match) {
    throw new Error(`missing-input:${step}`);
  }
  await match.locator.fill(value, { timeout: 5000 });
  await appendStep(task, `${step}:${match.selector}`);
  return true;
}

async function findVisibleLocator(page, selectors) {
  for (const selector of selectors) {
    const locator = page.locator(selector);
    const count = await locator.count();
    for (let index = count - 1; index >= 0; index -= 1) {
      const candidate = locator.nth(index);
      if (await candidate.isVisible().catch(() => false)) {
        return { locator: candidate, selector };
      }
    }
  }
  return null;
}

async function latestStorageState(profileDir) {
  try {
    const entries = await readdir(profileDir);
    const stateFiles = entries
      .filter((entry) => entry.endsWith(".storage-state.json"))
      .sort();
    const latest = stateFiles.at(-1);
    return latest ? path.join(profileDir, latest) : null;
  } catch {
    return null;
  }
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://${request.headers.host ?? "localhost"}`);

  if (request.method === "GET" && url.pathname === "/health") {
    const chromiumExecutable = await resolveChromiumExecutable();
    return json(response, 200, {
      service: "browser-assist",
      status: "ok",
      idleTtlMs,
      chromiumExecutable,
      profileRoot,
      directProxyConfigured: Boolean(directProxyUrl),
      warpProxyConfigured: Boolean(warpProxyUrl),
      activeTasks: [...tasks.values()].filter((task) => task.status === "running").length,
      queuedTasks: [...tasks.values()].filter((task) => task.status === "queued").length
    });
  }

  if (request.method === "GET" && url.pathname === "/v1/tasks") {
    return json(response, 200, {
      items: [...tasks.values()]
        .sort((a, b) => b.createdAt.localeCompare(a.createdAt))
        .map((task) => ({
          id: task.id,
          kind: task.kind,
          accountId: task.accountId,
          accountLabel: task.accountLabel,
          provider: task.provider,
          routeMode: task.routeMode,
          status: task.status,
          createdAt: task.createdAt,
          updatedAt: task.updatedAt,
          notes: task.notes,
          profileDir: task.profileDir,
          screenshotPath: task.screenshotPath,
          storageStatePath: task.storageStatePath,
          finalUrl: task.finalUrl,
          lastError: task.lastError,
          steps: task.steps
        }))
    });
  }

  if (request.method === "POST" && url.pathname === "/v1/tasks/login") {
    const body = await readJson(request);
    return json(response, 202, {
      task: enqueueTask("login", body)
    });
  }

  if (request.method === "POST" && url.pathname === "/v1/tasks/recover") {
    const body = await readJson(request);
    return json(response, 202, {
      task: enqueueTask("recover", body)
    });
  }

  return json(response, 404, {
    error: "not_found"
  });
});

await mkdir(profileRoot, { recursive: true });

server.listen(port, "0.0.0.0", () => {
  console.log(`browser-assist listening on :${port}`);
});
