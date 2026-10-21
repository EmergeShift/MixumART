export type Platform = 'macos' | 'windows' | 'linux'

export function detectPlatform(): Platform {
  const ua = navigator.userAgent
  if (/Mac|iPhone|iPad/.test(ua)) return 'macos'
  if (/Win/.test(ua)) return 'windows'
  return 'linux'
}
