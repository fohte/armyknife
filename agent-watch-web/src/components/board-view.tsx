import '#components/board-view.css'

import { Button } from '@fohte/ui/button'
import { useEffect, useRef, useState } from 'react'

import type { BoardProject } from '#board-model'
import { SessionPausedBadge } from '#components/session-paused-badge'

type BoardViewProps = {
  projects: readonly BoardProject[]
  loaded: boolean
  error: string | null
  hideQuietProjects: boolean
  onHideQuietProjectsChange: (checked: boolean) => void
  onSelectSession: (id: string) => void
  focusedProjectName?: string | null
}

const GROUPS = [
  {
    category: 'needs_action',
    title: 'Needs action',
    className: 'needs-action',
  },
  { category: 'in_progress', title: 'In progress', className: 'in-progress' },
  { category: 'done', title: 'Done', className: 'done' },
] as const

function formatElapsedTime(totalSeconds: number) {
  if (totalSeconds < 60) {
    return `${String(totalSeconds)}s`
  }

  const minutes = Math.floor(totalSeconds / 60)
  if (minutes < 60) {
    return `${String(minutes)}m`
  }

  const hours = Math.floor(minutes / 60)
  if (hours < 24) {
    return `${String(hours)}h`
  }

  return `${String(Math.floor(hours / 24))}d`
}

function cardKind(session: BoardProject['sessions'][number]) {
  if (session.category === 'in_progress') {
    return 'Working'
  }
  if (session.category === 'done') {
    return session.pullRequest?.state === 'Merged'
      ? `Merged #${String(session.pullRequest.number)}`
      : 'Done'
  }
  return session.kind ?? 'Stopped'
}

function taskLabel(session: BoardProject['sessions'][number]) {
  if (session.task === null) {
    return '— No task'
  }

  const priority =
    session.task.priority === null ? '—' : `P${String(session.task.priority)}`
  return `${priority} #${String(session.task.number)} · ${session.task.title}`
}

function ProjectCard({
  session,
  onSelectSession,
}: {
  session: BoardProject['sessions'][number]
  onSelectSession: (id: string) => void
}) {
  return (
    <Button
      type="button"
      variant="plain"
      className={`board-card block w-full cursor-pointer category-${session.category} hover:text-foreground`}
      data-kind={session.kind?.toLowerCase() ?? ''}
      aria-label={`Open in Inbox: ${session.title}`}
      onClick={() => {
        onSelectSession(session.id)
      }}
    >
      <div className="board-card-heading">
        <SessionPausedBadge isPaused={session.isPaused} />
        <span>{cardKind(session)}</span>
        <time>{formatElapsedTime(session.elapsedSeconds)}</time>
      </div>
      <div className="board-card-title">{session.title}</div>
      <div className="board-card-task">{taskLabel(session)}</div>
    </Button>
  )
}

function ProjectColumn({
  project,
  onSelectSession,
}: {
  project: BoardProject
  onSelectSession: (id: string) => void
}) {
  return (
    <section className="board-project" aria-label={project.name}>
      <header className="board-project-header">
        <div className="board-project-title-line">
          <h2>{project.name}</h2>
          {project.needsActionCount > 0 ? (
            <span className="board-project-need">
              {project.needsActionCount} need action
            </span>
          ) : (
            <span className="board-project-quiet">Nothing needed</span>
          )}
        </div>
        {project.repos.length > 0 ? (
          <p className="board-project-repos">{project.repos.join(', ')}</p>
        ) : null}
      </header>
      {GROUPS.map(({ category, title, className }) => {
        const sessions = project.sessions.filter(
          (session) => session.category === category,
        )
        if (sessions.length === 0) {
          return null
        }

        if (category === 'done') {
          const pausedSessions = sessions.filter((session) => session.isPaused)
          const otherDoneCount = sessions.length - pausedSessions.length

          return (
            <section
              key={category}
              className={`board-group ${className}`}
              aria-label={`${title} (${String(sessions.length)})`}
            >
              <h3>
                <span>{title}</span>
                <span>{sessions.length}</span>
              </h3>
              <p className="board-done-summary">
                {pausedSessions.length > 0 ? (
                  <>
                    {pausedSessions.map((session) => (
                      <span
                        className="board-done-paused-session"
                        key={session.id}
                      >
                        <SessionPausedBadge isPaused />
                        <span>{session.title}</span>
                      </span>
                    ))}
                    {otherDoneCount > 0
                      ? ` · ${String(otherDoneCount)} other done`
                      : ''}
                  </>
                ) : (
                  <>
                    {sessions[0]?.title ?? ''}
                    {sessions.length > 1
                      ? ` and ${String(sessions.length - 1)} more`
                      : ''}
                  </>
                )}
              </p>
            </section>
          )
        }

        const visibleSessions = sessions.slice(0, 3)

        return (
          <section
            key={category}
            className={`board-group ${className}`}
            aria-label={`${title} (${String(sessions.length)})`}
          >
            <h3>
              <span>{title}</span>
              <span>{sessions.length}</span>
            </h3>
            <div className="board-card-list">
              {visibleSessions.map((session) => (
                <ProjectCard
                  key={session.id}
                  session={session}
                  onSelectSession={onSelectSession}
                />
              ))}
              {sessions.length > visibleSessions.length ? (
                <p className="board-more">
                  {sessions.length - visibleSessions.length} more
                </p>
              ) : null}
            </div>
          </section>
        )
      })}
    </section>
  )
}

export function BoardView({
  projects,
  loaded,
  error,
  hideQuietProjects,
  onHideQuietProjectsChange,
  onSelectSession,
  focusedProjectName = null,
}: BoardViewProps) {
  const projectsScrollRef = useRef<HTMLDivElement>(null)
  const [remainingProjects, setRemainingProjects] = useState(0)
  const quietCount = projects.filter((project) => project.quiet).length
  const focusedProject = projects.filter(
    (project) => project.name === focusedProjectName,
  )
  const visibleProjects =
    focusedProjectName !== null
      ? focusedProject
      : hideQuietProjects
        ? projects.filter((project) => !project.quiet)
        : projects

  useEffect(() => {
    const scroller = projectsScrollRef.current
    if (scroller === null) {
      return
    }

    const updateRemainingProjects = () => {
      const rightEdge = scroller.scrollLeft + scroller.clientWidth
      const offscreenProjects = Array.from(
        scroller.querySelectorAll<HTMLElement>('.board-project'),
      ).filter(
        (project) => project.offsetLeft + project.clientWidth > rightEdge,
      )
      setRemainingProjects(offscreenProjects.length)
    }

    updateRemainingProjects()
    scroller.addEventListener('scroll', updateRemainingProjects)
    const resizeObserver = new ResizeObserver(updateRemainingProjects)
    resizeObserver.observe(scroller)
    const projects = scroller.querySelector('.board-projects')
    if (projects !== null) {
      resizeObserver.observe(projects)
    }

    return () => {
      scroller.removeEventListener('scroll', updateRemainingProjects)
      resizeObserver.disconnect()
    }
  }, [visibleProjects])

  return (
    <section className="board-view" aria-labelledby="board-title">
      <header className="board-header">
        <div className="board-header-title">
          <h1 id="board-title">Board</h1>
          <span>Projects that need you come first</span>
        </div>
        <label className="board-quiet-toggle">
          <input
            type="checkbox"
            checked={hideQuietProjects}
            onChange={(event) => {
              onHideQuietProjectsChange(event.currentTarget.checked)
            }}
          />
          <span>Hide quiet projects ({quietCount})</span>
        </label>
      </header>
      {error !== null ? (
        <p className="board-message is-error" role="status">
          {error}
        </p>
      ) : null}
      {!loaded ? (
        error === null ? (
          <p className="board-message" role="status">
            Loading projects…
          </p>
        ) : null
      ) : visibleProjects.length === 0 ? (
        <p className="board-message">No projects to show.</p>
      ) : (
        <div className="board-projects-scroll" ref={projectsScrollRef}>
          <div className="board-projects">
            {visibleProjects.map((project) => (
              <ProjectColumn
                key={project.name}
                project={project}
                onSelectSession={onSelectSession}
              />
            ))}
          </div>
          {remainingProjects > 0 ? (
            <div className="board-overflow-hint" aria-hidden="true">
              {remainingProjects} more →
            </div>
          ) : null}
        </div>
      )}
    </section>
  )
}
