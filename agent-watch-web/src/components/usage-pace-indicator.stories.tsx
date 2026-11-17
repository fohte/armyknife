import type { Meta, StoryObj } from '@storybook/react-vite'

import { UsagePaceIndicator } from '#components/usage-pace-indicator'

const nowSeconds = 1_800_000_000

const meta = {
  component: UsagePaceIndicator,
  args: {
    window: {
      usedPercentage: 30,
      resetsAt: nowSeconds + 9_000,
      windowSeconds: 18_000,
    },
    nowSeconds,
  },
  decorators: [
    (Story) => (
      <div
        style={{
          background: '#171717',
          fontFamily: 'var(--font-mono)',
          fontSize: '12px',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof UsagePaceIndicator>

export default meta
type Story = StoryObj<typeof meta>

export const OnPace: Story = {
  name: 'Usage is projected to stay below the quota',
}

export const ApproachingLimit: Story = {
  name: 'Usage is projected to approach the quota',
  args: {
    window: {
      usedPercentage: 70,
      resetsAt: nowSeconds + 9_000,
      windowSeconds: 18_000,
    },
  },
}

export const LikelyToExceedLimit: Story = {
  name: 'Usage is projected to exceed the quota',
  args: {
    window: {
      usedPercentage: 80,
      resetsAt: nowSeconds + 9_000,
      windowSeconds: 18_000,
    },
  },
}
