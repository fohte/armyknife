import type { Meta, StoryObj } from '@storybook/react-vite'

import { buildBoardProjects } from '#board-model'
import { BoardView } from '#components/board-view'
import type { Session, SessionTask } from '#types'

const baseSession: Session = {
  id: 'session-template',
  category: 'done',
  isPaused: false,
  kind: null,
  title: 'Document the deployment process',
  preview: null,
  elapsedSeconds: 3600,
  unread: false,
  task: null,
  repo: 'example/alpha',
  project: 'alpha suite',
  ancestorSessionIds: [],
  parent: null,
  pullRequest: null,
}

function createSession(
  session: Pick<Session, 'id' | 'category' | 'project' | 'title'> &
    Partial<Session>,
): Session {
  return { ...baseSession, ...session }
}

function createTask(id: string, number: number, title: string): SessionTask {
  return { id, number, title, priority: 1 }
}

const sessions: Session[] = [
  ...Array.from({ length: 5 }, (_, index) =>
    createSession({
      id: `alpha-action-${String(index)}`,
      category: 'needs_action',
      project: 'alpha suite',
      kind:
        ['Question', 'Review', 'Error', 'Approve', 'Plan'][index] ?? 'Review',
      title:
        [
          'Confirm the delivery retry policy',
          'Review the retry implementation',
          'Resolve the failed integration check',
          'Choose the notification policy',
          'Plan the next rollout',
        ][index] ?? 'Review the retry implementation',
      elapsedSeconds: 8 + index * 120,
      unread: index === 0,
      task: createTask('task-delivery', 2048, 'Improve delivery reliability'),
      repo: index === 1 ? 'example/queue' : 'example/alpha',
    }),
  ),
  ...Array.from({ length: 6 }, (_, index) =>
    createSession({
      id: `alpha-working-${String(index)}`,
      category: 'in_progress',
      project: 'alpha suite',
      title:
        index % 2 === 0 ? 'Add delivery metrics' : 'Improve queue throughput',
      elapsedSeconds: 30 + index * 60,
      task: createTask('task-delivery', 2048, 'Improve delivery reliability'),
    }),
  ),
  ...Array.from({ length: 3 }, (_, index) =>
    createSession({
      id: `alpha-done-${String(index)}`,
      category: 'done',
      project: 'alpha suite',
      title:
        [
          'Document delivery limits',
          'Add a queue health check',
          'Update service examples',
        ][index] ?? 'Document delivery limits',
      pullRequest: index === 0 ? { number: 2051, state: 'Merged' } : null,
    }),
  ),
  createSession({
    id: 'beta-question',
    category: 'needs_action',
    project: 'beta tools',
    repo: 'example/beta',
    kind: 'Question',
    title: 'Choose the command output format',
    elapsedSeconds: 1200,
    task: createTask('task-cli', 2052, 'Improve command output'),
  }),
  ...Array.from({ length: 2 }, (_, index) =>
    createSession({
      id: `beta-working-${String(index)}`,
      category: 'in_progress',
      project: 'beta tools',
      repo: 'example/beta',
      title: index === 0 ? 'Add table output' : 'Document output options',
      task: createTask('task-cli', 2052, 'Improve command output'),
    }),
  ),
  createSession({
    id: 'beta-done',
    category: 'done',
    project: 'beta tools',
    repo: 'example/beta',
    title: 'Add the JSON output option',
  }),
  ...['gamma docs', 'delta site'].map((project, index) =>
    createSession({
      id: `quiet-${String(index)}`,
      category: 'done',
      project,
      repo: index === 0 ? 'example/docs' : 'example/site',
      title:
        index === 0 ? 'Refresh the usage guide' : 'Update the release page',
    }),
  ),
  ...Array.from({ length: 2 }, (_, index) =>
    createSession({
      id: `epsilon-working-${String(index)}`,
      category: 'in_progress',
      project: 'epsilon service',
      repo: 'example/epsilon',
      title: index === 0 ? 'Improve request tracing' : 'Reduce startup time',
    }),
  ),
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
    onSelectSession: () => undefined,
  },
  decorators: [
    (Story) => (
      <div
        className="story-board-container"
        style={{ display: 'flex', width: 1200, height: 900 }}
      >
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

export const BoardShowsPausedSessions: Story = {
  name: 'The Done group identifies paused sessions',
  args: {
    projects: buildBoardProjects([
      ...sessions,
      {
        id: 'session-paused',
        category: 'done',
        isPaused: true,
        kind: null,
        title: 'Prepare the next deployment window',
        preview: null,
        elapsedSeconds: 240,
        unread: false,
        task: null,
        repo: 'example/service',
        project: 'service refresh',
        ancestorSessionIds: [],
        parent: null,
        pullRequest: null,
      },
    ]),
  },
}

export const BoardShowsQuietProjects: Story = {
  name: 'The board includes quiet projects when the filter is off',
  args: { hideQuietProjects: false },
  parameters: {
    overflowCheck: { ignoreSelectors: ['.board-projects-scroll'] },
  },
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
