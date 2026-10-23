// 与 crates/mixum-env-core 的 serde 输出一一对应。
// 改这里的任何字段，先改 core 的类型，再同步本文件。

export type RuntimeKind = 'docker' | 'wsl2' | 'apple_container'

export type RuntimeStatus =
  | { state: 'ready'; version: string | null; detail: string | null }
  | { state: 'not_installed'; hint: string }
  | { state: 'not_running'; hint: string }
  | { state: 'unsupported'; reason: string }

export type BackendStatus = {
  kind: RuntimeKind
  name: string
  status: RuntimeStatus
}

export type ContainerState = 'created' | 'running' | 'paused' | 'stopped' | 'unknown'

export type PortMapping = {
  host: number
  container: number
  protocol: string
}

/** 该容器由谁发起、以哪种协作模式工作（Agent 平台接入的归属信息）。 */
export type AgentMode = 'as-tool' | 'sub-agent' | 'isolated'

export type AgentRef = {
  name: string
  mode: AgentMode
  core?: string
}

export type Container = {
  id: string
  name: string
  image: string
  state: ContainerState
  status: string | null
  createdAt: string | null
  ports: PortMapping[]
  /** 后端尚未提供时为空——界面按「未标注」渲染，不虚构。 */
  agent?: AgentRef | null
}

/** 容器对应 MAP 节点的连接档案；令牌由 Rust 侧持久化，不写 localStorage。 */
export type EnvironmentConnection = {
  addr: string
  token: string
  fingerprint?: string | null
}

/** 前端可见的连接信息；token 留在 Rust/钥匙串，不下发 WebView。 */
export type ConnectionInfo = {
  addr: string
  fingerprint?: string | null
}

export type CreateSpec = {
  name: string
  image: string
  ports?: PortMapping[]
  mounts?: unknown[]
  env?: Record<string, string>
  command?: string[]
}

export type SnapshotNode = {
  id: string
  parent?: string | null
  branch: string
  label?: string | null
  kind: string
  createdAt?: string | null
  sizeBytes?: number
  chunks?: number
}

export type SnapshotTree = {
  head?: string | null
  branch?: string | null
  nodes?: SnapshotNode[]
}

export type HostInfo = {
  os: string
  arch: string
  appVersion: string
}

export interface SurfaceInfo {
  id: number
  appId: string
  title: string
  width: number
  height: number
}

export interface SurfaceFrame {
  surface: number
  fullW: number
  fullH: number
  x: number
  y: number
  w: number
  h: number
  format: number
  /** PNG，base64 编码：Tauri 事件是 JSON，数字数组会有约 4 倍体积。 */
  data: string
}

export type SurfaceInputEvent =
  | { type: 'pointer_motion'; surface: number; x: number; y: number }
  | { type: 'pointer_button'; surface: number; button: 'left' | 'right' | 'middle'; action: 'press' | 'release' }
  | { type: 'pointer_axis'; surface: number; action: 'scroll_up' | 'scroll_down' }
  | { type: 'key'; surface: number; keysym: string; action: 'press' | 'release' }
  | { type: 'text'; surface: number; text: string }
