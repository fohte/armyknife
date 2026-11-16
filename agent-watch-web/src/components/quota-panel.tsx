import '#components/quota-panel.css'

import {
  type EngineQuota,
  formatResetTime,
  getExpectedUsagePercentage,
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
  const usedPercentage =
    window === null ? null : Math.round(window.usedPercentage)
  const actualPercentage = window?.usedPercentage ?? null
  const expectedPercentage =
    window === null ? null : getExpectedUsagePercentage(window, nowSeconds)
  const isAhead =
    actualPercentage !== null &&
    expectedPercentage !== null &&
    actualPercentage > expectedPercentage
  const color =
    usedPercentage === null
      ? '#d4d4d8'
      : usedPercentage >= 95
        ? '#f87171'
        : usedPercentage >= 80
          ? '#fbbf24'
          : '#d4d4d8'
  const reset =
    window === null ? '--' : formatResetTime(window.resetsAt, nowSeconds)
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
        {expectedPercentage === null ? null : (
          <span
            aria-label={`${isAhead ? 'Ahead of pace' : 'On pace'}: ${String(Math.round(expectedPercentage))}% expected by now`}
            className={`quota-window-mark${isAhead ? ' is-ahead' : ''}`}
            title={`${isAhead ? 'Ahead of pace' : 'On pace'}: ${String(Math.round(expectedPercentage))}% expected by now`}
          >
            {isAhead ? '▲' : '✓'}
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
      {error && quotas === null ? (
        <span className="quota-panel-error" role="status">
          Usage unavailable
        </span>
      ) : null}
    </section>
  )
}
