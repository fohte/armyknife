import { requestJson } from '#api-requests'

export type RateLimitWindow = {
  usedPercentage: number
  resetsAt: number | null
  windowSeconds: number
}

type RateLimits = {
  fiveHour: RateLimitWindow | null
  sevenDay: RateLimitWindow | null
}

export type SessionUsage = {
  sessionId: string
  engine: 'claude' | 'codex'
  model: string | null
  effort: string | null
  context: {
    usedTokens: number
    windowTokens: number
    usedPercentage: number
  } | null
  rateLimits: RateLimits
  version: string | null
  observedAt: string
}

export type EngineQuota = {
  engine: 'claude' | 'codex'
  rateLimits: RateLimits
  observedAt: string
}

export type AgentUsage = {
  sessions: SessionUsage[]
  quotas: {
    claude: EngineQuota | null
    codex: EngineQuota | null
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isRateLimitWindow(value: unknown): value is RateLimitWindow {
  return (
    isRecord(value) &&
    typeof value['usedPercentage'] === 'number' &&
    (typeof value['resetsAt'] === 'number' || value['resetsAt'] === null) &&
    typeof value['windowSeconds'] === 'number'
  )
}

function isRateLimits(value: unknown): value is RateLimits {
  return (
    isRecord(value) &&
    (value['fiveHour'] === null || isRateLimitWindow(value['fiveHour'])) &&
    (value['sevenDay'] === null || isRateLimitWindow(value['sevenDay']))
  )
}

function isSessionUsage(value: unknown): value is SessionUsage {
  return (
    isRecord(value) &&
    typeof value['sessionId'] === 'string' &&
    (value['engine'] === 'claude' || value['engine'] === 'codex') &&
    (typeof value['model'] === 'string' || value['model'] === null) &&
    (typeof value['effort'] === 'string' || value['effort'] === null) &&
    (value['context'] === null ||
      (isRecord(value['context']) &&
        typeof value['context']['usedTokens'] === 'number' &&
        typeof value['context']['windowTokens'] === 'number' &&
        typeof value['context']['usedPercentage'] === 'number')) &&
    isRateLimits(value['rateLimits']) &&
    (typeof value['version'] === 'string' || value['version'] === null) &&
    typeof value['observedAt'] === 'string'
  )
}

function isEngineQuota(value: unknown): value is EngineQuota {
  return (
    isRecord(value) &&
    (value['engine'] === 'claude' || value['engine'] === 'codex') &&
    isRateLimits(value['rateLimits']) &&
    typeof value['observedAt'] === 'string'
  )
}

export function isAgentUsage(value: unknown): value is AgentUsage {
  return (
    isRecord(value) &&
    Array.isArray(value['sessions']) &&
    value['sessions'].every(isSessionUsage) &&
    isRecord(value['quotas']) &&
    (value['quotas']['claude'] === null ||
      isEngineQuota(value['quotas']['claude'])) &&
    (value['quotas']['codex'] === null ||
      isEngineQuota(value['quotas']['codex']))
  )
}

export function requestAgentUsage() {
  return requestJson('/api/usage', isAgentUsage)
}

export function getExpectedUsagePercentage(
  window: RateLimitWindow,
  nowSeconds: number,
) {
  if (window.resetsAt === null || window.windowSeconds <= 0) return null
  const elapsedRatio =
    (window.windowSeconds - (window.resetsAt - nowSeconds)) /
    window.windowSeconds
  if (elapsedRatio < 0.05) return null
  return Math.max(0, Math.min(100, elapsedRatio * 100))
}

export function getUsagePaceStatus(
  usedPercentage: number,
  expectedPercentage: number | null,
) {
  if (expectedPercentage === null) return null
  return usedPercentage > expectedPercentage ? 'ahead' : 'on-pace'
}

export function getUsageSeverity(usedPercentage: number | null) {
  if (usedPercentage === null || usedPercentage < 80) return 'normal'
  if (usedPercentage < 95) return 'warning'
  return 'danger'
}

export function formatTokenCount(tokens: number) {
  if (tokens < 1_000) return String(tokens)
  const value = tokens / 1_000
  return `${Number.isInteger(value) ? String(value) : value.toFixed(1)}k`
}

export function formatResetTime(resetsAt: number | null, nowSeconds: number) {
  if (resetsAt === null) return '--'
  const remainingSeconds = Math.max(0, resetsAt - nowSeconds)
  const minutes = Math.floor(remainingSeconds / 60)
  const days = Math.floor(minutes / 1_440)
  const hours = Math.floor((minutes % 1_440) / 60)
  const remainingMinutes = minutes % 60
  if (days > 0) return `${String(days)}d${String(hours)}h`
  if (hours > 0) return `${String(hours)}h${String(remainingMinutes)}m`
  return `${String(remainingMinutes)}m`
}
