// MixumART 真实 gateway 端到端:起 gateway + 一个节点,签发节点令牌,
// 再跑 Rust 集成测试(crates/mixum-env-map/tests/gateway_live.rs)。
//
// 前置:docker、mixumos-e2e-rust:latest 镜像、mixum-gateway 静态二进制
//   (默认取 MIXUM_GATEWAY_BIN，否则 ../.. 下 mixumOS fork 的 build/)
// 用法: node tests/gateway-live.mjs
process.env.MSYS_NO_PATHCONV = "1";
process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = fileURLToPath(new URL("..", import.meta.url)).replace(/\\/g, "/");
const PORT = 17701;
const GW = "mixum-gw-live";
const NODE = "mixum-node-live";
const NODE_NAME = "a";
const IMAGE = "mixumos-e2e-rust:latest";
const BIN =
  process.env.MIXUM_GATEWAY_BIN ??
  join(
    dirname(dirname(ROOT)),
    "2026-10-16",
    "8720bdc7",
    "mixumOS-yubing-dev-fork",
    "build",
    "mixum-gateway-x64",
  );

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function sh(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: "utf8", ...opts });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}
function docker(args) {
  return sh("docker", args, { env: process.env });
}
function mustDocker(args) {
  const r = docker(args);
  if (r.status !== 0) throw new Error(`docker ${args.join(" ")}: ${r.out.slice(-400)}`);
  return r.out;
}
function cleanup() {
  docker(["rm", "-f", GW]);
  docker(["rm", "-f", NODE]);
}

if (!existsSync(BIN)) throw new Error(`缺少 gateway 二进制: ${BIN}`);

try {
  cleanup();
  const binDir = dirname(BIN);
  mustDocker([
    "run", "-d", "--rm", "--name", GW, "-p", `${PORT}:17701`, "-v",
    `${binDir}:/opt:ro`, "alpine:3.21", "/opt/mixum-gateway-x64",
    "--listen", "wss://0.0.0.0:17701", "--data-dir", "/tmp/gw",
  ]);
  let admin, fp;
  for (let i = 0; i < 40 && !admin; i++) {
    await sleep(250);
    const logs = docker(["logs", GW]).out;
    admin = logs.match(/bootstrap token: (\S+)/)?.[1] ?? "";
    fp = logs.match(/fingerprint: (\S+)/)?.[1] ?? fp;
  }
  if (!admin) throw new Error("gateway 未就绪");
  console.log("gateway 就绪（bootstrap + fingerprint）");

  // admin 会话签发节点令牌。
  const socket = new WebSocket(`wss://127.0.0.1:${PORT}/ws`);
  const pending = new Map();
  socket.addEventListener("message", (ev) => {
    if (typeof ev.data !== "string") return;
    const env = JSON.parse(ev.data);
    const w = pending.get(env.replyTo);
    if (w) {
      pending.delete(env.replyTo);
      w(env);
    }
  });
  await new Promise((res, rej) => {
    socket.addEventListener("open", res, { once: true });
    socket.addEventListener("error", () => rej(new Error("gateway 连接失败")), { once: true });
  });
  const call = (op, payload) =>
    new Promise((res, rej) => {
      const env = { v: 1, id: crypto.randomUUID(), op, ts: Date.now(), payload };
      const t = setTimeout(() => rej(new Error(`超时 ${op}`)), 10000);
      pending.set(env.id, (reply) => {
        clearTimeout(t);
        reply.op.endsWith(".err")
          ? rej(new Error(`${op}: ${reply.payload?.message}`))
          : res(reply.payload);
      });
      socket.send(JSON.stringify(env));
    });
  await call("hello", { token: admin, client: "mixumart-e2e/0.1" });
  const issued = await call("gateway.token.issue", { kind: "node", name: NODE_NAME });
  const before = await call("gateway.node.list", {});
  console.log("节点令牌签发 ✓，当前节点:", JSON.stringify(before.nodes));
  socket.close();

  // 节点容器:出站注册到 gateway(wss 自签,relay 走 insecure_tls_client)。
  mustDocker([
    "run", "-d", "--rm", "--name", NODE,
    "-e", `MIXUM_GATEWAY_URL=wss://host.docker.internal:${PORT}`,
    "-e", `MIXUM_GATEWAY_NAME=${NODE_NAME}`,
    "-e", `MIXUM_GATEWAY_TOKEN=${issued.token}`,
    IMAGE,
  ]);
  for (let i = 0; i < 60; i++) {
    await sleep(500);
    const logs = docker(["logs", NODE]).out;
    if (logs.includes("READY token=")) break;
  }
  console.log("节点容器 READY ✓（等待注册与窗口）");
  await sleep(3000);

  const res = spawnSync(
    "cargo",
    ["test", "-p", "mixum-env-map", "--test", "gateway_live", "--", "--ignored", "--nocapture"],
    {
      cwd: ROOT,
      stdio: "inherit",
      env: {
        ...process.env,
        MAP_GATEWAY_URL: `wss://127.0.0.1:${PORT}/ws`,
        MAP_GATEWAY_TOKEN: admin,
        ...(fp ? { MAP_GATEWAY_FP: fp } : {}),
        MAP_NODE_NAME: NODE_NAME,
      },
    },
  );
  if (res.status !== 0) throw new Error(`Rust 集成测试失败 exit=${res.status}`);
  console.log("PASS MixumART gateway 实际链路（节点/配对/设备/吊销）");
} finally {
  cleanup();
}
