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

export type Container = {
  id: string
  name: string
  image: string
  state: ContainerState
  status: string | null
  createdAt: string | null
  ports: PortMapping[]
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
  data: number[]
}

export type SurfaceInputEvent =
  | { type: 'pointer_motion'; surface: number; x: number; y: number }
  | { type: 'pointer_button'; surface: number; button: 'left' | 'right' | 'middle'; action: 'press' | 'release' }
  | { type: 'key'; surface: number; keysym: string; action: 'press' | 'release' }
