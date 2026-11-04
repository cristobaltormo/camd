import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export async function launch({ port, width = 1280, height = 900, extraArgs = [] }) {
  const profile = mkdtempSync(join(tmpdir(), "camd-chrome-"));
  const args = [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--hide-scrollbars",
    "--remote-allow-origins=*",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    `--window-size=${width},${height}`,
    ...extraArgs,
    "about:blank",
  ];
  const chrome = spawn(process.env.CHROMIUM || "chromium", args, { stdio: ["ignore", "ignore", "pipe"] });
  let stderr = "";
  chrome.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-4000); });
  let exited = null;
  chrome.on("exit", (code) => { exited = code; });

  let target;
  for (let i = 0; i < 160 && !target; i++) {
    if (exited !== null) throw new Error(`browser exited with ${exited}\n${stderr}`);
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      target = list.find((item) => item.type === "page")?.webSocketDebuggerUrl;
    } catch (_) {}
    if (!target) await sleep(250);
  }
  if (!target) throw new Error(`browser did not start\n${stderr}`);

  const ws = new WebSocket(target);
  await new Promise((resolve, reject) => { ws.addEventListener("open", resolve); ws.addEventListener("error", () => reject(new Error("devtools connection failed"))); });
  let nextId = 0;
  const waiting = new Map();
  const listeners = [];
  ws.addEventListener("message", (message) => {
    const data = JSON.parse(message.data);
    if (data.id && waiting.has(data.id)) { waiting.get(data.id)(data); waiting.delete(data.id); return; }
    listeners.forEach((fn) => fn(data));
  });
  const send = (method, params = {}) => new Promise((resolve) => { const id = ++nextId; waiting.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); });
  const js = async (expression) => {
    const reply = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (reply.result.exceptionDetails) throw new Error(reply.result.exceptionDetails.exception?.description || "evaluate failed");
    return reply.result.result.value;
  };
  const close = async () => {
    ws.close();
    chrome.kill();
    await sleep(600);
    try { rmSync(profile, { recursive: true, force: true, maxRetries: 5 }); } catch (_) {}
  };
  await send("Runtime.enable");
  await send("Page.enable");
  return { send, js, close, onEvent: (fn) => listeners.push(fn) };
}
