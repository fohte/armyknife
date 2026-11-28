import type { Meta, StoryObj } from '@storybook/react-vite'

import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import { ProposalCard } from '#components/proposal-card'
import type { ProposalOption, SessionLinkTarget } from '#types'

const linkedSession: SessionLinkTarget = {
  id: '11111111-1111-4111-8111-111111111111',
  repo: 'example/toolkit',
  task: null,
  title: 'Inspect the retry path',
}

const options: ProposalOption[] = [
  {
    label: 'Use the existing flow',
    description: 'Keep the current sequence and add a short explanation.',
    pros: ['Small change'],
    cons: ['Adds one step'],
    recommended: true,
  },
  {
    label: 'Show the choice earlier',
    description: 'Move the decision before the next action.',
    pros: ['Clearer timing'],
    cons: ['Changes the current flow'],
    recommended: false,
  },
]

const callbacks = {
  onAnswer: () => Promise.resolve(true),
}

const meta = {
  component: ProposalCard,
  args: {
    seq: 1,
    title: 'When should the new setting take effect?',
    summary: 'Choose how the setting should be introduced.',
    options,
    answer: null,
    resolved: false,
    resolution: null,
    isMobile: false,
    ...callbacks,
  },
} satisfies Meta<typeof ProposalCard>

export default meta
type Story = StoryObj<typeof meta>

export const ProposalWaitsForAChoice: Story = {
  name: 'A proposal waits for a choice',
}

export const ProposalFormatsItsAgentTextAsMarkdown: Story = {
  name: 'A proposal formats its summary and option descriptions as markdown',
  args: {
    summary: 'Compare **the options** in [the guide](https://example.test).',
    options: [
      {
        label: 'Use the existing flow',
        description:
          'Keep **the current flow** and [review details](https://example.test).',
        pros: ['Small change'],
        cons: ['Adds one step'],
        recommended: true,
      },
      ...options.slice(1),
    ],
  },
}

export const ProposalChoiceShowsCodeBlock: Story = {
  name: 'A proposal option shows a code block with the copy toolbar',
  args: {
    options: options.map((option, index) =>
      index === 0
        ? {
            ...option,
            description: '~~~bash\nprintf "ready\\n"\n~~~',
          }
        : option,
    ),
  },
}

export const ProposalLinksKnownSessionIds: Story = {
  name: 'A known session ID links from the summary and option description',
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
    summary: `Compare the proposal with ${linkedSession.id}.`,
    options: options.map((option, index) =>
      index === 0
        ? {
            ...option,
            description: `Use the behavior from ${linkedSession.id}.`,
          }
        : option,
    ),
  },
}

export const ProposalShowsASelectedChoice: Story = {
  name: 'A proposal marks the selected choice',
  args: { defaultChoice: 1 },
}

export const ProposalShowsACommentWithoutAChoice: Story = {
  name: 'A proposal shows a comment without a selected choice',
  args: { defaultComment: 'Consider the effect on existing users.' },
}

export const ProposalOffersTouchSizedChoicesOnMobile: Story = {
  name: 'A proposal offers touch sized choices on mobile',
  args: { isMobile: true, defaultChoice: 0 },
  decorators: [
    (Story) => (
      <div style={{ width: 390, padding: 16 }}>
        <Story />
      </div>
    ),
  ],
}

export const ProposalShowsAChosenAnswer: Story = {
  name: 'A proposal shows the choice selected by the user',
  args: {
    answer: {
      choice: 0,
      comment: 'Keep the change small.',
    },
    resolved: true,
  },
}

export const MobileProposalShowsAChosenAnswer: Story = {
  name: 'A mobile proposal shows the selected option after answering',
  args: {
    isMobile: true,
    answer: {
      choice: 0,
      comment: 'Keep the change small.',
    },
    resolved: true,
  },
}

export const ProposalShowsAnAttachedImage: Story = {
  name: 'A proposal shows its attached image with the saved answer',
  args: {
    answer: {
      choice: 0,
      comment: '![Sample screenshot](/card-comment-attachment.svg)',
    },
    resolved: true,
  },
}

export const ProposalShowsAnotherOptionWasRequested: Story = {
  name: 'A proposal shows that another option was requested',
  args: {
    answer: { choice: null, comment: 'Please include a simpler option.' },
    resolved: true,
  },
}

export const ProposalShowsItWasAnsweredElsewhere: Story = {
  name: 'A proposal shows that it was answered elsewhere',
  args: { resolved: true },
}

export const ProposalShowsItWasWithdrawn: Story = {
  name: 'A withdrawn proposal shows only its heading and title',
  args: {
    resolved: true,
    resolution: 'withdrawn',
  },
}
