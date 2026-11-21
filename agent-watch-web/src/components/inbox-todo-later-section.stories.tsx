import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxTodoLaterSection } from '#components/inbox-todo-later-section'
import type { Session } from '#types'

function foldedTodoSession(
  id: string,
  title: string,
  count: number,
  firstTodoTitle: string,
): Session {
  return {
    id,
    category: 'done',
    isPaused: false,
    kind: 'Todo',
    title,
    preview: firstTodoTitle,
    elapsedSeconds: 0,
    unread: false,
    task: {
      id: 'sample-task',
      number: 7,
      title: 'Example task',
      priority: 1,
    },
    repo: 'example/service',
    project: 'sample project',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
    todoLaterCount: count,
    todoLaterTitle: firstTodoTitle,
  }
}

const sessions = [
  foldedTodoSession(
    'sample-alert-session',
    'Check a phone alert',
    2,
    'Check the alert sound',
  ),
  foldedTodoSession(
    'sample-delivery-session',
    'Verify a delivery',
    1,
    'Check the delivery status',
  ),
]

const meta = {
  component: InboxTodoLaterSection,
  args: {
    sessions,
    selectedId: null,
    onSelect: () => undefined,
    isMobile: false,
  },
  decorators: [
    (Story) => (
      <div className="story-inbox-container">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof InboxTodoLaterSection>

export default meta
type Story = StoryObj<typeof meta>

export const SummarizesSeveralFoldedTodos: Story = {
  name: 'The row summarizes folded to-dos across sessions',
}

export const ListsEachSessionWhenExpanded: Story = {
  name: 'The expanded row lists each session with folded to-dos',
  args: { expanded: true },
}

export const OpensTheOnlySession: Story = {
  name: 'A single folded to-do session stays summarized',
  args: { sessions: sessions.slice(0, 1) },
}
