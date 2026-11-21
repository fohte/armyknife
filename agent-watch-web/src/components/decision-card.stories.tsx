import type { Meta, StoryObj } from '@storybook/react-vite'

import { type Decision, DecisionCard } from '#components/decision-card'
import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import type { SessionLinkTarget } from '#types'

const linkedSession: SessionLinkTarget = {
  id: '11111111-1111-4111-8111-111111111111',
  repo: 'example/toolkit',
  task: null,
  title: 'Inspect the retry path',
}

const decision: Decision = {
  type: 'decision',
  seq: 1,
  title: 'Use a fixed retry limit',
  why: 'The current service has no reliable timing data.',
  considered: ['Retry using the latest duration', 'Retry once per package'],
  easyToChange: 'One configuration value in the CI workflow.',
  review: null,
  createdAt: '2026-11-11T03:25:59Z',
}

const meta = {
  component: DecisionCard,
  args: {
    decision,
    variant: 'conversation',
    relativeTimeReference: Date.parse('2026-11-11T04:00:00Z'),
    onOk: () => Promise.resolve(true),
    onRequestChange: () => Promise.resolve(true),
  },
  decorators: [
    (Story) => (
      <div
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: '720px',
          padding: '16px',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DecisionCard>

export default meta
type Story = StoryObj<typeof meta>

export const ConversationRowIsCollapsed: Story = {
  name: 'The conversation keeps a decision in one collapsed row',
}

export const ConversationRowIsExpanded: Story = {
  name: 'The expanded conversation row shows the decision details',
  args: { expanded: true },
}

export const DecisionMarkdownLinksKnownSessionIds: Story = {
  name: 'Decision details link known session IDs in every markdown field',
  decorators: [
    (Story) => (
      <MarkdownSessionLinksProvider
        sessions={[linkedSession]}
        onSelectSession={() => undefined}
      >
        <Story />
      </MarkdownSessionLinksProvider>
    ),
  ],
  args: {
    expanded: true,
    decision: {
      ...decision,
      why: `Follow the approach from ${linkedSession.id}.`,
      considered: [
        `Keep the behavior in ${linkedSession.id}.`,
        'Use a separate implementation.',
      ],
      easyToChange: `Revisit the setting with ${linkedSession.id}.`,
    },
  },
}

export const MobileConversationRowKeepsALongTitleReadable: Story = {
  name: 'The mobile conversation row keeps a long title readable',
  args: {
    decision: {
      ...decision,
      title: 'Keep retries within a predictable time budget',
    },
    position: '131 of 132',
  },
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          padding: 16,
        }}
      >
        <Story />
      </div>
    ),
  ],
}

export const ExpandedDecisionActionsOnIPhone: Story = {
  name: 'The expanded iPhone decision shows touch sized review actions',
  args: { expanded: true },
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          padding: 16,
        }}
      >
        <Story />
      </div>
    ),
  ],
}

export const SelectedConversationRowShowsItsPosition: Story = {
  name: 'The selected conversation row shows its position in the list',
  args: { expanded: true, selected: true, position: '1 of 2' },
}

export const SelectedPanelCardIsExpanded: Story = {
  name: 'The selected panel card shows its details and review actions',
  args: {
    variant: 'panel',
    expanded: true,
    selected: true,
    targetName: 'the parent session',
  },
}

export const LongTargetNameFitsPanelActions: Story = {
  name: 'A long target session name is truncated while the shortcut stays visible',
  args: {
    variant: 'panel',
    expanded: true,
    selected: true,
    targetName:
      'a long target session name for an asynchronous refactoring run',
  },
  decorators: [
    (Story) => (
      <div style={{ width: '280px', padding: '16px' }}>
        <Story />
      </div>
    ),
  ],
}

export const OtherPanelCardIsCollapsed: Story = {
  name: 'An unselected panel card shows its title and alternatives',
  args: { variant: 'panel', targetName: 'this session' },
}

export const ChangeCommentIsBeingWritten: Story = {
  name: 'The selected card has an open change request field',
  args: {
    variant: 'panel',
    expanded: true,
    selected: true,
    editing: true,
    targetName: 'the parent session',
  },
}

export const DecisionWasApproved: Story = {
  name: 'The expanded decision shows that it was approved',
  args: {
    decision: { ...decision, review: 'ok' },
    expanded: true,
  },
}
