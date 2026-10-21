// 浏览器里直接打开 Vite 页面时没有 Tauri 后端：用演示数据渲染完整界面，
// 用于设计评审与截图。顶栏会明确标注「演示数据」，不伪装成真实状态。
import type { BackendStatus, Container, HostInfo, SurfaceInfo } from './types'

export const isDemo = () =>
  typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)

export const demoHost: HostInfo = {
  os: 'macos',
  arch: 'arm64',
  appVersion: '0.2.0-demo',
}

export const demoBackends: BackendStatus[] = [
  {
    kind: 'apple_container',
    name: 'Apple Container',
    status: { state: 'ready', version: '1.5.0', detail: null },
  },
  {
    kind: 'docker',
    name: 'Docker',
    status: { state: 'not_running', hint: 'Docker Desktop 未启动' },
  },
  {
    kind: 'wsl2',
    name: 'WSL 2',
    status: { state: 'unsupported', reason: '仅 Windows 宿主可用' },
  },
]

export const demoContainers: Container[] = [
  {
    id: '01jbv9c2q7r9s1t3u5v7w9x2y4',
    name: 'agent-codex-billing',
    image: 'mixumos:0.1.4',
    state: 'running',
    status: 'Up 2 hours',
    createdAt: '2026-10-20T08:12:00Z',
    ports: [{ host: 17610, container: 7601, protocol: 'tcp' }],
    agent: { name: 'Codex', mode: 'as-tool', core: 'none' },
  },
  {
    id: '01jbv9k4m2p6t8v0x3z5b7d9f1h',
    name: 'agent-claude-notes',
    image: 'mixumos:0.1.4',
    state: 'running',
    status: 'Up 48 minutes',
    createdAt: '2026-10-20T09:31:00Z',
    ports: [{ host: 17611, container: 7601, protocol: 'tcp' }],
    agent: { name: 'Claude Code', mode: 'sub-agent', core: 'mixum' },
  },
  {
    id: '01jbv9p7r4s8u1w3y6a9c2e5g7j',
    name: 'scratch-fedora-build',
    image: 'mixumos:0.1.3',
    state: 'stopped',
    status: 'Exited (0) 3 hours ago',
    createdAt: '2026-10-20T06:02:00Z',
    ports: [],
    agent: null,
  },
  {
    id: '01jbv9t3x6z9b2d4f7h0k3m5n8q',
    name: 'agent-cursor-frontend',
    image: 'mixumos:0.1.4',
    state: 'paused',
    status: 'Paused',
    createdAt: '2026-10-20T07:45:00Z',
    ports: [{ host: 17612, container: 7601, protocol: 'tcp' }],
    agent: { name: 'Cursor', mode: 'isolated', core: 'mixum' },
  },
]

export const demoSurfaces: SurfaceInfo[] = [
  { id: 1, appId: 'foot', title: 'agent-codex-billing — shell', width: 996, height: 663 },
  { id: 1000, appId: 'codex-cli', title: 'Codex session · billing', width: 1280, height: 800 },
]
