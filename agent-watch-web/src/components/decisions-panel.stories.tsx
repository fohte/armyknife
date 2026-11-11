import type { Meta, StoryObj } from '@storybook/react-vite'

import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import type { Session } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'done',
  isPaused: false,
  kind: null,
  title: 'Finish the retry policy update',
  preview: null,
  elapsedSeconds: 120,
  unread: false,
  task: null,
  repo: 'example/service',
  project: 'service reliability',
  parent: { id: 'parent-session', title: 'Service reliability work' },
  ancestorSessionIds: ['parent-session'],
  pullRequest: null,
}

const decisions: Decision[] = [
  {
    type: 'decision',
    seq: 1,
    title: 'Use a fixed retry limit',
    why: 'The current service has no reliable timing data.',
    considered: ['Retry using the latest duration', 'Retry once per package'],
    easyToChange: 'One configuration value in the CI workflow.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  },
  {
    type: 'decision',
    seq: 2,
    title: 'Keep the current backoff interval',
    why: 'It already matches the service limits.',
    considered: ['Use a shorter interval'],
    easyToChange: 'The interval is one configuration value.',
    review: 'ok',
    createdAt: '2026-11-11T03:27:59Z',
  },
]

const meta = {
  component: DecisionsPanel,
  args: {
    decisions,
    session,
    selectedSeq: 1,
    relativeTimeReference: Date.parse('2026-11-11T04:00:00Z'),
    onSelect: () => undefined,
    onOk: () => Promise.resolve(true),
    onChange: () => Promise.resolve(true),
  },
  decorators: [
    (Story) => (
      <div style={{ width: '460px', height: '800px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DecisionsPanel>

export default meta
type Story = StoryObj<typeof meta>

export const PendingDecisionIsSelected: Story = {
  name: 'The panel expands the selected decision and shows review progress',
}

export const AnotherDecisionIsSelected: Story = {
  name: 'The panel keeps other decisions collapsed around the selection',
  args: { selectedSeq: 2 },
}

export const ChangeIsSentToTheSession: Story = {
  name: 'An active session shows itself as the change request target',
  args: {
    session: { ...session, category: 'in_progress', parent: null },
  },
}

export const EveryDecisionIsReviewed: Story = {
  name: 'The panel shows the reviewed count when no action is pending',
  args: {
    decisions: decisions.map((decision) => ({ ...decision, review: 'ok' })),
    selectedSeq: 2,
  },
}
