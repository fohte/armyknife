import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxSessionList } from '#components/inbox-session-list'
import type { Session } from '#types'

function sampleSession(
  id: string,
  project: string,
  priority: number,
  updatedAt: string,
): Session {
  return {
    id,
    category: 'needs_action',
    isPaused: false,
    kind: 'Question',
    title: `Review changes for ${project}`,
    preview: 'A recent update is waiting for a response.',
    updatedAt,
    elapsedSeconds: 60,
    unread: true,
    task: {
      id: `task-${id}`,
      number: 17,
      title: 'Review the changes',
      priority,
    },
    repo: `sample-org/${project}`,
    project,
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  }
}

const alphaPriorityOne = sampleSession(
  'sample-alpha-priority-one',
  'alpha',
  1,
  '2030-01-01T12:00:00Z',
)
const alphaPriorityTwo = sampleSession(
  'sample-alpha-priority-two',
  'alpha',
  2,
  '2030-01-01T11:00:00Z',
)
const betaPriorityOne = sampleSession(
  'sample-beta-priority-one',
  'beta',
  1,
  '2030-01-01T10:00:00Z',
)
const projectSortedSessions = [
  alphaPriorityOne,
  alphaPriorityTwo,
  betaPriorityOne,
]
const prioritySortedSessions = [
  betaPriorityOne,
  alphaPriorityOne,
  alphaPriorityTwo,
]

const meta = {
  component: InboxSessionList,
  args: {
    sessions: prioritySortedSessions,
    selectedId: null,
    onSelect: () => undefined,
    isMobile: false,
  },
  decorators: [
    (Story, context) =>
      context.args.isMobile ? (
        <div
          className="story-mobile-layout"
          data-layout="mobile"
          style={{ width: 390, height: 844 }}
        >
          <Story />
        </div>
      ) : (
        <div
          className="story-inbox-container"
          data-layout="desktop"
          style={{ minHeight: 220 }}
        >
          <Story />
        </div>
      ),
  ],
} satisfies Meta<typeof InboxSessionList>

export default meta
type Story = StoryObj<typeof meta>

export const PrioritySortKeepsAFlatList: Story = {
  name: 'Priority sort keeps the session list flat',
}

export const ProjectSortGroupsSessionsByProject: Story = {
  name: 'Project sort groups sessions and keeps priority within each group',
  args: { sessions: projectSortedSessions, groupByProject: true },
}

export const IPhoneProjectSortShowsProjectHeadings: Story = {
  name: 'The iPhone shows project headings above each group',
  args: {
    sessions: projectSortedSessions,
    groupByProject: true,
    isMobile: true,
  },
}
