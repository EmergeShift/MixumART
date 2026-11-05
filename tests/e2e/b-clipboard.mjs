#!/usr/bin/env node
// B 路线（Smithay 合成器）剪贴板 e2e。
//
// 覆盖 RFC 0004 的 compositor contract：合成器连上 agentd 的控制 socket 后要
// 声明 capabilities{clipboard:true}，宿主的 display.clipboard.set 要落到容器
// 的 seat selection（Wayland 与 X11 两侧），容器内客户端复制要上行成
// display.clipboard.changed 并带上正确的窗口归属。
//
// 用法：node tests/e2e/b-clipboard.mjs [port]
// 需要：docker；镜像默认 mixum-b-clip-e2e:test（本脚本自己 build）。
// 二进制默认取 build/mixumd、build/mixum-env-compositor，可用环境变量覆盖。
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..", "..");
const NAME = process.env.MIXUM_E2E_NAME ?? "mixum-bclip";
const PORT = Number(process.argv[2] ?? 17611);
const IMAGE = process.env.MIXUM_E2E_IMAGE ?? "mixum-b-clip-e2e:test";
const MIXUMD = process.env.MIXUMD_BIN ?? join(ROOT, "build", "mixumd");
const COMPOSITOR =
  process.env.COMPOSITOR_BIN ?? join(ROOT, "build", "mixum-env-compositor");
const LIMIT = 4 << 20;

// docker exec 不继承 entrypoint 的环境，手动补 Wayland / X11 两套变量。
const ENV =
  "export XDG_RUNTIME_DIR=/run/mixum WAYLAND_DISPLAY=mixum-art-0; " +
  'export DISPLAY=":$(cat /run/mixum/xwayland.display 2>/dev/null | tr -d \'\\n\')";';

function docker(args, options = {}) {
  return spawnSync("docker", args, { encoding: "utf8", ...options });
}

function sh(cmd, options = {}) {
  return execFileSync("docker", ["exec", NAME, "sh", "-c", ENV + cmd], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  }).replace(/\r?\n$/, "");
}

function execDetached(cmd) {
  execFileSync("docker", ["exec", "-d", NAME, "sh", "-c", ENV + cmd], { stdio: "ignore" });
}

function buildImage() {
  if (process.env.MIXUM_E2E_SKIP_BUILD === "1") return;
  for (const [label, path] of [
    ["mixumd", MIXUMD],
    ["mixum-env-compositor", COMPOSITOR],
  ]) {
    if (!existsSync(path)) {
      console.error(`缺少 ${label}：${path}`);
      process.exit(2);
    }
  }
  const context = mkdtempSync(join(tmpdir(), "mixum-b-clip-"));
  const assets = join(HERE, "b-clipboard");
  for (const file of ["Dockerfile", "entrypoint.sh"]) {
    copyFileSync(join(assets, file), join(context, file));
  }
  copyFileSync(MIXUMD, join(context, "mixumd"));
  copyFileSync(COMPOSITOR, join(context, "mixum-env-compositor"));
  const built = docker(["build", "-q", "-t", IMAGE, context]);
  rmSync(context, { recursive: true, force: true });
  if (built.status !== 0) {
    console.error(built.stdout + built.stderr);
    process.exit(1);
  }
}

function frame(op, payload) {
  return JSON.stringify({
    v: 1,
    id: crypto.randomUUID(),
    op,
    ts: Date.now(),
    ...(payload ? { payload } : {}),
  });
}

const session = {
  ws: null,
  pending: new Map(),
  clips: [],
  windows: [],
  async connect(url) {
    this.ws = new WebSocket(url);
    this.ws.addEventListener("message", (event) => {
      if (typeof event.data !== "string") return;
      const env = JSON.parse(event.data);
      const waiter = this.pending.get(env.replyTo);
      if (waiter) {
        this.pending.delete(env.replyTo);
        waiter(env);
        return;
      }
      // display.windows 是订阅：窗口以推送形式来，不是应答里的列表。
      if (env.op === "display.windows") {
        this.windows.push(env.payload ?? {});
      }
      if (env.op === "display.clipboard.changed") {
        this.clips.push({ window: env.payload?.window, text: env.payload?.text ?? "" });
      }
    });
    await new Promise((resolveOpen, rejectOpen) => {
      this.ws.addEventListener("open", resolveOpen, { once: true });
      this.ws.addEventListener("error", () => rejectOpen(new Error("ws 连接失败")), {
        once: true,
      });
    });
  },
  call(op, payload) {
    return new Promise((resolveCall, rejectCall) => {
      const raw = frame(op, payload);
      const id = JSON.parse(raw).id;
      const timer = setTimeout(() => rejectCall(new Error(`超时 ${op}`)), 15000);
      this.pending.set(id, (env) => {
        clearTimeout(timer);
        env.op.endsWith(".err")
          ? rejectCall(Object.assign(new Error(env.payload?.message), { code: env.payload?.code }))
          : resolveCall(env.payload);
      });
      this.ws.send(raw);
    });
  },
  async expectError(op, payload, code, label) {
    try {
      await this.call(op, payload);
      throw new Error(`${label}：应当失败却成功了`);
    } catch (error) {
      if (error.code !== code) throw new Error(`${label}：期望 ${code}，实际 ${error.code} (${error.message})`);
    }
  },
  nextClip(timeoutMs = 5000) {
    return new Promise((resolveClip, rejectClip) => {
      const started = Date.now();
      const poll = () => {
        if (this.clips.length) {
          resolveClip(this.clips.shift());
          return;
        }
        if (Date.now() - started > timeoutMs) {
          rejectClip(new Error("没有收到 display.clipboard.changed"));
          return;
        }
        setTimeout(poll, 50);
      };
      poll();
    });
  },
  close() {
    try {
      this.ws.close();
    } catch {
      /* noop */
    }
  },
};

function ok(label) {
  console.log(`  ok: ${label}`);
}

async function waitFor(predicate, timeoutMs, label) {
  const started = Date.now();
  for (;;) {
    // 谓词可以是 async：发现 B 窗口要发一次 display.windows。
    const value = await predicate();
    if (value) return value;
    if (Date.now() - started > timeoutMs) throw new Error(`超时：${label}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

/// 同 waitFor，但超时返回 null 而不是抛错（可选用例，例如镜像里没有 Xwayland）。
async function waitForOptional(predicate, timeoutMs) {
  const started = Date.now();
  for (;;) {
    const value = await predicate();
    if (value) return value;
    if (Date.now() - started > timeoutMs) return null;
    await new Promise((r) => setTimeout(r, 100));
  }
}

buildImage();

const debugEnv = process.env.MIXUM_MAP_DEBUG ? ["-e", "MIXUM_MAP_DEBUG=1"] : [];
docker(["rm", "-f", NAME]);
docker(["run", "-d", "--rm", "--name", NAME, ...debugEnv, "-p", `${PORT}:7601`, IMAGE]);

let token = "";
try {
  token = await waitFor(
    () => docker(["logs", NAME]).stdout.match(/READY token=(\S+)/)?.[1],
    40000,
    "容器 READY",
  );
  ok("容器就绪");
  // 等合成器把 capabilities{clipboard:true} 送到 agentd。
  await new Promise((r) => setTimeout(r, 1500));

  const s = session;
  await s.connect(`wss://127.0.0.1:${PORT}/ws`);
  await s.call("hello", { token, client: "b-clipboard-e2e/0.1" });
  ok("admin 会话就绪");

  // display.windows 只是订阅；窗口以 display.windows 事件推送过来。
  await s.call("display.windows", {});
  // 开一个真实的 Wayland 窗口：上行归属与 X11 都要靠它。
  execDetached("exec foot --override=main.initial-window-size-pixels=600x400 --hold cat /etc/os-release");
  const window = await waitFor(
    () => s.windows.find((item) => (item.id ?? 0) >= 1000)?.id ?? null,
    25000,
    "B 窗口出现",
  );
  ok(`B 窗口 window=${window}`);

  // 1) 显式订阅门控：没 subscribe 之前，容器复制不应产生事件。
  const pre = `b-pre-${crypto.randomUUID().slice(0, 8)}`;
  sh(`printf %s '${pre}' | wl-copy`);
  await new Promise((r) => setTimeout(r, 1200));
  if (s.clips.length) throw new Error("未订阅就收到了 clipboard.changed");
  ok("未订阅前不推送");

  await s.call("display.clipboard.subscribe", {});
  ok("display.clipboard.subscribe");

  // 2) 下行：宿主 set → 容器 wl-paste 读到原文。
  const down = `b-down-${crypto.randomUUID().slice(0, 8)} 中文 ✓`;
  await s.call("display.clipboard.set", { window, text: down });
  const pasted = sh("wl-paste --no-newline");
  if (pasted !== down) throw new Error(`下行不一致：${JSON.stringify(pasted)}`);
  ok("set → 容器 wl-paste");

  // 3) 上行：容器 wl-copy → changed 事件，且窗口归属正确。
  const up = `b-up-${crypto.randomUUID().slice(0, 8)} 上行`;
  sh(`printf %s '${up}' | wl-copy`);
  const changed = await s.nextClip();
  if (changed.text !== up) throw new Error(`上行文本不一致：${JSON.stringify(changed.text)}`);
  if (changed.window !== window) {
    throw new Error(`上行窗口归属错误：${changed.window} != ${window}`);
  }
  ok("容器复制 → changed（窗口归属正确）");

  // 4) 边界：空串、超限都拒绝；恰好 4 MiB 通过。
  await s.expectError("display.clipboard.set", { window, text: "" }, "bad_request", "空串");
  ok("空串被拒");
  await s.expectError(
    "display.clipboard.set",
    { window, text: "a".repeat(LIMIT + 1) },
    "bad_request",
    "超限",
  );
  ok("4 MiB+1 被拒");
  const exact = "a".repeat(LIMIT);
  await s.call("display.clipboard.set", { window, text: exact });
  const bytes = Number(sh("wl-paste --no-newline | wc -c"));
  if (bytes !== LIMIT) throw new Error(`4 MiB 边界：容器读到 ${bytes} 字节`);
  ok("恰好 4 MiB 通过");

  // 5) X11 侧（可选，默认跳过）。X11 读写依赖 Xwayland + XWM 的选区代理，
  //    在最小镜像里时序不稳（xclip 用 CurrentTime 抢所有权会被 X server 拒绝），
  //    所以默认不跑；需要时 MIXUM_E2E_X11=1 打开。
  const x11 = `b-x11-${crypto.randomUUID().slice(0, 8)}`;
  const hasX11 = process.env.MIXUM_E2E_X11 === "1" && (await waitForOptional(() => {
    try {
      return sh("test -f /run/mixum/xwayland.display && xdpyinfo >/dev/null 2>&1 && echo up");
    } catch {
      return null;
    }
  }, 20000));
  if (hasX11) {
    s.clips.length = 0;
    sh(`printf %s '${x11}' | xclip -selection clipboard`);
    // 先确认容器里 X11 选区真的设上了，再等上行事件——不然超时看不出是
    // "xclip 没抢到选区" 还是 "合成器没上报"。只比长度，别把内容打进日志。
    const x11Owner = sh("timeout 3 xclip -selection clipboard -o");
    if (x11Owner !== x11) {
      throw new Error(`X11 选区没设上：读到 ${x11Owner.length} 字节`);
    }
    const event = await s.nextClip(10000);
    if (event.text !== x11) throw new Error(`X11 上行不一致：${JSON.stringify(event.text)}`);
    ok("xclip 复制 → changed（X11 → 宿主）");
    // 宿主 → X11 的下行还没做：合成器故意不抢 X11 选区所有权——一旦抢了，X11
    // 应用再复制就收不到 XFixes 事件，整条上行链路会断（实测过）。见
    // docs/display.md 的已知缺口。
    console.log("  skip: 宿主 → X11 下行（当前覆盖 Wayland 侧，见 docs/display.md）");
  } else {
    console.log("  skip: X11 用例默认关闭（MIXUM_E2E_X11=1 打开，见 docs/display.md）");
  }

  console.log("b-clipboard-e2e PASS");
  s.close();
} catch (error) {
  console.error(`FAILED: ${error.message}`);
  const logs = docker(["logs", "--tail", "60", NAME]).stdout;
  console.error(logs);
  process.exitCode = 1;
} finally {
  docker(["rm", "-f", NAME]);
}
