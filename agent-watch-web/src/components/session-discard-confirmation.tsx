import '#components/session-discard-confirmation.css'

import { useId, useState } from 'react'

import { ConfirmDialog } from '#components/confirm-dialog'

type DiscardFileChange = {
  status: 'M' | 'A' | 'D' | 'R' | '??'
  path: string
  isNew?: boolean
}

type DiscardCommit = {
  sha: string
  subject: string
  isNew?: boolean
}

type SessionDiscardConfirmationProps = {
  sessionName: string
  branchName: string
  worktreePath: string
  changes: readonly DiscardFileChange[]
  commits: readonly DiscardCommit[]
  reviewAgain?: boolean
  mobile?: boolean
  onCancel: () => void
  onDiscard: () => void
}

const ITEMS_PER_SECTION = 5

function formatCount(count: number, singular: string, plural: string) {
  return `${String(count)} ${count === 1 ? singular : plural}`
}

function formatNewItems(fileCount: number, commitCount: number) {
  return [
    fileCount > 0 ? formatCount(fileCount, 'new file', 'new files') : null,
    commitCount > 0
      ? formatCount(commitCount, 'new commit', 'new commits')
      : null,
  ]
    .filter((item) => item !== null)
    .join(' and ')
}

function ordered<T extends { isNew?: boolean }>(items: readonly T[]) {
  return [
    ...items.filter((item) => item.isNew === true),
    ...items.filter((item) => item.isNew !== true),
  ]
}

export function SessionDiscardConfirmation({
  sessionName,
  branchName,
  worktreePath,
  changes,
  commits,
  reviewAgain = false,
  mobile = false,
  onCancel,
  onDiscard,
}: SessionDiscardConfirmationProps) {
  const [showAllChanges, setShowAllChanges] = useState(false)
  const [showAllCommits, setShowAllCommits] = useState(false)
  const changesHeadingId = useId()
  const changesListId = useId()
  const commitsHeadingId = useId()
  const commitsListId = useId()
  const newFileCount = changes.filter(({ isNew }) => isNew === true).length
  const newCommitCount = commits.filter(({ isNew }) => isNew === true).length
  const title = reviewAgain
    ? 'Review the changes again'
    : 'Discard changes and close?'
  const newItems = formatNewItems(newFileCount, newCommitCount)
  const description = reviewAgain
    ? newItems.length > 0
      ? `Nothing was deleted: ${newItems} appeared after you confirmed. Discarding deletes these too.`
      : 'Nothing was deleted. Review the current changes before discarding them.'
    : `Deletes the worktree and the local branch ${branchName}. Changes that are not saved elsewhere will be lost.`

  return (
    <ConfirmDialog
      className={`session-discard-confirmation${mobile ? ' session-discard-confirmation--mobile' : ''}`}
      title={title}
      description={
        <>
          <span className="session-discard-confirmation__meta">
            <span className="session-discard-confirmation__failed">
              Close failed
            </span>
            <strong title={sessionName}>{sessionName}</strong>
          </span>
          <span className="session-discard-confirmation__description">
            {description}
          </span>
        </>
      }
      confirmLabel="Discard and close"
      confirmVariant="danger"
      mobile={mobile}
      trapFocus={mobile}
      onCancel={onCancel}
      onConfirm={onDiscard}
    >
      <div className="session-discard-confirmation__items">
        {changes.length > 0 ? (
          <section aria-labelledby={changesHeadingId}>
            <div className="session-discard-confirmation__section-heading">
              <h3 id={changesHeadingId}>Uncommitted changes</h3>
              <span>{formatCount(changes.length, 'file', 'files')}</span>
            </div>
            <ul id={changesListId}>
              {ordered(changes)
                .slice(0, showAllChanges ? undefined : ITEMS_PER_SECTION)
                .map(({ status, path, isNew = false }) => (
                  <li
                    className={
                      isNew
                        ? 'session-discard-confirmation__new-item'
                        : undefined
                    }
                    key={`${status}:${path}`}
                  >
                    <code
                      className={`session-discard-confirmation__status session-discard-confirmation__status--${status === '??' ? 'untracked' : status.toLowerCase()}`}
                    >
                      {status}
                    </code>
                    <code className="session-discard-confirmation__path">
                      {path}
                    </code>
                    {isNew ? (
                      <span className="session-discard-confirmation__new-badge">
                        New
                      </span>
                    ) : null}
                  </li>
                ))}
            </ul>
            {changes.length > ITEMS_PER_SECTION ? (
              <button
                aria-controls={changesListId}
                aria-expanded={showAllChanges}
                className="session-discard-confirmation__show-more"
                onClick={() => {
                  setShowAllChanges((expanded) => !expanded)
                }}
                type="button"
              >
                {showAllChanges
                  ? 'Show less'
                  : `Show ${String(changes.length - ITEMS_PER_SECTION)} more`}
              </button>
            ) : null}
          </section>
        ) : null}
        {commits.length > 0 ? (
          <section aria-labelledby={commitsHeadingId}>
            <div className="session-discard-confirmation__section-heading">
              <h3 id={commitsHeadingId}>Commits not on main</h3>
              <span>{formatCount(commits.length, 'commit', 'commits')}</span>
            </div>
            <ul id={commitsListId}>
              {ordered(commits)
                .slice(0, showAllCommits ? undefined : ITEMS_PER_SECTION)
                .map(({ sha, subject, isNew = false }) => (
                  <li
                    className={
                      isNew
                        ? 'session-discard-confirmation__new-item'
                        : undefined
                    }
                    key={sha}
                  >
                    <code>{sha}</code>
                    <span className="session-discard-confirmation__subject">
                      {subject}
                    </span>
                    {isNew ? (
                      <span className="session-discard-confirmation__new-badge">
                        New
                      </span>
                    ) : null}
                  </li>
                ))}
            </ul>
            {commits.length > ITEMS_PER_SECTION ? (
              <button
                aria-controls={commitsListId}
                aria-expanded={showAllCommits}
                className="session-discard-confirmation__show-more"
                onClick={() => {
                  setShowAllCommits((expanded) => !expanded)
                }}
                type="button"
              >
                {showAllCommits
                  ? 'Show less'
                  : `Show ${String(commits.length - ITEMS_PER_SECTION)} more`}
              </button>
            ) : null}
          </section>
        ) : null}
      </div>
      <code className="session-discard-confirmation__worktree-path">
        {worktreePath}
      </code>
    </ConfirmDialog>
  )
}
