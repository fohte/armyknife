import type { Meta, StoryObj } from '@storybook/react-vite'

import { buildBoardProjects } from '#board-model'
import { BoardView } from '#components/board-view'
import type { Session } from '#types'

const sessions: Session[] = [
  {
    id: 'session-question',
    category: 'needs_action',
    kind: 'Question',
    title: 'Review the updated API contract',
    preview: 'Can the response include a project name?',
    elapsedSeconds: 8,
    unread: true,
    task: {
      id: 'task-api-contract',
      number: 2048,
      title: 'Update the service contract',
      priority: 1,
    },
    repo: 'example/service',
    project: 'service refresh',
    parent: null,
    pullRequest: null,
  },
  {
    id: 'session-working',
    category: 'in_progress',
    kind: null,
    title: 'Add request metrics',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'service refresh',
    parent: null,
    pullRequest: null,
  },
  {
    id: 'session-done',
    category: 'done',
    kind: null,
    title: 'Document request limits',
    preview: null,
    elapsedSeconds: 3600,
    unread: false,
    task: null,
    repo: 'example/service',
    project: 'service refresh',
    parent: null,
    pullRequest: { number: 2049, state: 'Merged' },
  },
  {
    id: 'session-quiet',
    category: 'done',
    kind: null,
    title: 'Record deployment notes',
    preview: null,
    elapsedSeconds: 7200,
    unread: false,
    task: null,
    repo: 'example/docs',
    project: 'documentation update',
    parent: null,
    pullRequest: null,
  },
]

const projects = buildBoardProjects(sessions)

const meta = {
  component: BoardView,
  args: {
    projects,
    loaded: true,
    error: null,
    hideQuietProjects: true,
    onHideQuietProjectsChange: () => undefined,
  },
  decorators: [
    (Story) => (
      <div className="story-board-container">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof BoardView>

export default meta
type Story = StoryObj<typeof meta>

export const BoardShowsActiveProjectsFirst: Story = {
  name: 'The board puts projects that need attention first',
}

export const BoardShowsQuietProjects: Story = {
  name: 'The board includes quiet projects when the filter is off',
  args: { hideQuietProjects: false },
}

export const BoardHasNoProjects: Story = {
  name: 'The board shows an empty state when no projects are available',
  args: { projects: [] },
}

export const BoardIsLoading: Story = {
  name: 'The board shows a loading state before projects arrive',
  args: { projects: [], loaded: false },
}

export const BoardShowsRefreshError: Story = {
  name: 'The board keeps showing projects when metadata refresh fails',
  args: { error: 'Project metadata could not be refreshed.' },
}
