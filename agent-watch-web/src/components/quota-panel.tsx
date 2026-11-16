import '#components/quota-panel.css'

import {
  type EngineQuota,
  formatResetTime,
  getExpectedUsagePercentage,
  getUsagePaceStatus,
  getUsageSeverity,
  type RateLimitWindow,
} from '#agent-usage'

type Quotas = {
  claude: EngineQuota | null
  codex: EngineQuota | null
}

type QuotaPanelProps = {
  quotas: Quotas | null
  error?: boolean
  isMobile?: boolean
  nowSeconds?: number
}

function QuotaWindow({
  engine,
  label,
  window,
  nowSeconds,
}: {
  engine: string
  label: string
  window: RateLimitWindow | null
  nowSeconds: number
}) {
  // A stored snapshot cannot tell us how much quota was used after its reset.
  const currentWindow =
    window !== null && window.resetsAt !== null && window.resetsAt <= nowSeconds
      ? null
      : window
  const usedPercentage =
    currentWindow === null ? null : Math.round(currentWindow.usedPercentage)
  const actualPercentage = currentWindow?.usedPercentage ?? null
  const expectedPercentage =
    currentWindow === null
      ? null
      : getExpectedUsagePercentage(currentWindow, nowSeconds)
  const paceStatus = getUsagePaceStatus(
    actualPercentage ?? 0,
    expectedPercentage,
  )
  const severity = getUsageSeverity(usedPercentage)
  const color =
    severity === 'danger'
      ? '#f87171'
      : severity === 'warning'
        ? '#fbbf24'
        : '#d4d4d8'
  const reset =
    currentWindow === null
      ? '--'
      : formatResetTime(currentWindow.resetsAt, nowSeconds)
  return (
    <div className="quota-window">
      <span>{label}</span>
      <span
        aria-label={`${engine} ${label} quota used`}
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={usedPercentage ?? 0}
        aria-valuetext={
          usedPercentage === null
            ? 'Unavailable'
            : `${String(usedPercentage)}% used`
        }
        className="quota-meter"
        role="meter"
      >
        {usedPercentage !== null ? (
          <span
            className="quota-meter-fill"
            style={{
              width: `${String(Math.max(0, Math.min(100, usedPercentage)))}%`,
              backgroundColor: color,
            }}
          />
        ) : null}
        {expectedPercentage !== null ? (
          <span
            aria-hidden="true"
            className="quota-meter-pace"
            style={{ left: `${String(expectedPercentage)}%` }}
          />
        ) : null}
      </span>
      <span className="quota-window-percentage" style={{ color }}>
        {usedPercentage === null ? '--' : `${String(usedPercentage)}%`}
      </span>
      <span className="quota-window-reset">
        {paceStatus === null ? null : (
          <span
            aria-label={`${paceStatus === 'ahead' ? 'Ahead of pace' : 'On pace'}: ${String(Math.round(expectedPercentage ?? 0))}% expected by now`}
            className={`quota-window-mark${paceStatus === 'ahead' ? ' is-ahead' : ''}`}
            title={`${paceStatus === 'ahead' ? 'Ahead of pace' : 'On pace'}: ${String(Math.round(expectedPercentage ?? 0))}% expected by now`}
          >
            {paceStatus === 'ahead' ? '▲' : '✓'}
          </span>
        )}{' '}
        {reset}
      </span>
    </div>
  )
}

export function QuotaPanel({
  quotas,
  error = false,
  isMobile = false,
  nowSeconds = Math.floor(Date.now() / 1_000),
}: QuotaPanelProps) {
  const engines = [
    { id: 'claude', title: 'Claude Code', quota: quotas?.claude ?? null },
    { id: 'codex', title: 'Codex', quota: quotas?.codex ?? null },
  ] as const
  return (
    <section
      aria-label="Quota"
      className="quota-panel"
      data-mobile={isMobile ? 'true' : 'false'}
    >
      {engines.map(({ id, title, quota }) => (
        <div className="quota-engine" key={id}>
          <span className="quota-engine-name" title={title}>
            {id}
          </span>
          <QuotaWindow
            engine={id}
            label="5h"
            nowSeconds={nowSeconds}
            window={quota?.rateLimits.fiveHour ?? null}
          />
          <QuotaWindow
            engine={id}
            label="7d"
            nowSeconds={nowSeconds}
            window={quota?.rateLimits.sevenDay ?? null}
          />
        </div>
      ))}
      {error ? (
        <span className="quota-panel-error" role="status">
          {quotas === null
            ? 'Usage unavailable'
            : 'Refresh failed; showing stale usage'}
        </span>
      ) : null}
    </section>
  )
}
