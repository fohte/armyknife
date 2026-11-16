import type { Meta, StoryObj } from '@storybook/react-vite'

import type { EngineQuota } from '#agent-usage'
import { QuotaPanel } from '#components/quota-panel'

const nowSeconds = 1_794_832_000

const claudeQuota: EngineQuota = {
  engine: 'claude',
  rateLimits: {
    fiveHour: {
      usedPercentage: 41,
      resetsAt: nowSeconds + 7_800,
      windowSeconds: 18_000,
    },
    sevenDay: {
      usedPercentage: 76,
      resetsAt: nowSeconds + 136_800,
      windowSeconds: 604_800,
    },
  },
  observedAt: '2026-11-16T12:00:00Z',
}

const codexQuota: EngineQuota = {
  engine: 'codex',
  rateLimits: {
    fiveHour: {
      usedPercentage: 88,
      resetsAt: nowSeconds + 2_820,
      windowSeconds: 18_000,
    },
    sevenDay: {
      usedPercentage: 35,
      resetsAt: nowSeconds + 360_000,
      windowSeconds: 604_800,
    },
  },
  observedAt: '2026-11-16T12:00:00Z',
}

const meta = {
  component: QuotaPanel,
  args: {
    quotas: { claude: claudeQuota, codex: codexQuota },
    nowSeconds,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '240px', background: '#171717' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof QuotaPanel>

export default meta
type Story = StoryObj<typeof meta>

export const BothEnginesShowTheirWindows: Story = {
  name: 'The panel shows 5h and 7d quota for both engines',
}

export const UsageAheadOfPaceIsHighlighted: Story = {
  name: 'The panel highlights usage that is ahead of pace',
  args: {
    quotas: {
      claude: {
        ...claudeQuota,
        rateLimits: {
          ...claudeQuota.rateLimits,
          fiveHour: {
            usedPercentage: 93,
            resetsAt: nowSeconds + 7_800,
            windowSeconds: 18_000,
          },
        },
      },
      codex: null,
    },
  },
}

export const QuotaHasNotBeenRecorded: Story = {
  name: 'The panel shows placeholders before quota is recorded',
  args: { quotas: null },
}

export const UsageRequestFailed: Story = {
  name: 'The panel explains when usage cannot be loaded',
  args: { quotas: null, error: true },
}

export const MobileQuotaPanel: Story = {
  name: 'The panel uses compact insets in the mobile drawer',
  args: { isMobile: true },
}
