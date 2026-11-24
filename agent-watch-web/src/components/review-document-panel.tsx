import '#components/review-document-panel.css'

import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { markdown } from '@codemirror/lang-markdown'
import { defaultHighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { Compartment, EditorState } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { Maximize2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import { ReviewDocumentNavigation } from '#components/review-document-navigation'
import { ReviewDocumentPreview } from '#components/review-document-preview'
import type { ReviewDocumentSummary } from '#components/review-document-summary'
import type { JsonValue } from '#types'

type ReviewDocumentPanelProps = {
  summary?: ReviewDocumentSummary
  summaries?: readonly ReviewDocumentSummary[]
  isMobile?: boolean
  canSave?: boolean
  canFinish?: boolean
  onSelectDocument?: (id: string) => void
  onOpenFocus?: () => void
  frontmatter: JsonValue
  body: string
  status?: 'idle' | 'saving' | 'saved' | 'error' | 'finishing'
  unsavedChanges?: boolean
  error?: string | null
  view?: 'edit' | 'preview'
  onFrontmatterChange?: (value: JsonValue) => void
  onBodyChange?: (value: string) => void
  onViewChange?: (view: 'edit' | 'preview') => void
  onSave?: () => void
  onFinish?: () => void
}

type JsonObject = { [key: string]: JsonValue }
type JsonPathPart = string | number

function isJsonObject(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function updateAtPath(
  value: JsonValue,
  path: readonly JsonPathPart[],
  nextValue: JsonValue,
): JsonValue {
  const [part, ...remaining] = path
  if (part === undefined) return nextValue
  if (Array.isArray(value) && typeof part === 'number') {
    return value.map((item, index) =>
      index === part ? updateAtPath(item, remaining, nextValue) : item,
    )
  }
  if (isJsonObject(value) && typeof part === 'string') {
    return {
      ...value,
      [part]: updateAtPath(value[part] ?? null, remaining, nextValue),
    }
  }
  return value
}

function NumberField({
  name,
  value,
  disabled,
  onChange,
}: {
  name: string
  value: number
  disabled: boolean
  onChange: (value: JsonValue) => void
}) {
  const [inputValue, setInputValue] = useState(String(value))

  useEffect(() => {
    setInputValue(String(value))
  }, [disabled, value])

  return (
    <input
      aria-label={name}
      className="review-document-input review-document-number-input"
      disabled={disabled}
      inputMode="decimal"
      type="text"
      value={inputValue}
      onBlur={() => {
        if (inputValue === '' || !Number.isFinite(Number(inputValue))) {
          setInputValue(String(value))
        }
      }}
      onChange={(event) => {
        const nextValue = event.currentTarget.value
        setInputValue(nextValue)
        if (nextValue !== '' && Number.isFinite(Number(nextValue))) {
          onChange(Number(nextValue))
        }
      }}
    />
  )
}

function FieldEditor({
  name,
  value,
  path,
  disabled,
  onChange,
}: {
  name: string
  value: JsonValue
  path: readonly JsonPathPart[]
  disabled: boolean
  onChange: (path: readonly JsonPathPart[], value: JsonValue) => void
}) {
  if (isJsonObject(value)) {
    return (
      <div className="review-document-field-group">
        <h3 className="review-document-group-title">{name}</h3>
        <dl className="review-document-fields review-document-fields--nested">
          {Object.entries(value).map(([key, nestedValue]) => (
            <FieldEditor
              key={key}
              name={key}
              path={[...path, key]}
              disabled={disabled}
              value={nestedValue}
              onChange={onChange}
            />
          ))}
        </dl>
      </div>
    )
  }

  if (Array.isArray(value)) {
    return (
      <div className="review-document-field-group">
        <dt className="review-document-group-title">{name}</dt>
        <dd className="review-document-group-content">
          <ol className="review-document-array">
            {value.map((item, index) => (
              <li key={index}>
                <dl className="review-document-fields">
                  <FieldEditor
                    name={`Item ${String(index + 1)}`}
                    path={[...path, index]}
                    disabled={disabled}
                    value={item}
                    onChange={onChange}
                  />
                </dl>
              </li>
            ))}
          </ol>
        </dd>
      </div>
    )
  }

  if (typeof value === 'boolean') {
    return (
      <div className="review-document-field">
        <dt className="review-document-field-key">{name}</dt>
        <dd className="review-document-field-value">
          <input
            aria-label={name}
            checked={value}
            className="review-document-checkbox"
            disabled={disabled}
            type="checkbox"
            onChange={(event) => {
              onChange(path, event.currentTarget.checked)
            }}
          />
        </dd>
      </div>
    )
  }

  const inputValue = value === null ? '' : String(value)
  return (
    <div className="review-document-field">
      <dt className="review-document-field-key">{name}</dt>
      <dd className="review-document-field-value">
        {typeof value === 'number' ? (
          <NumberField
            name={name}
            value={value}
            disabled={disabled}
            onChange={(nextValue) => {
              onChange(path, nextValue)
            }}
          />
        ) : (
          <input
            aria-label={name}
            className="review-document-input"
            disabled={disabled}
            type="text"
            value={inputValue}
            onChange={(event) => {
              onChange(path, event.currentTarget.value)
            }}
          />
        )}
      </dd>
    </div>
  )
}

function MarkdownEditor({
  value,
  disabled,
  onChange,
}: {
  value: string
  disabled: boolean
  onChange: (value: string) => void
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  const editorRef = useRef<EditorView | null>(null)
  const editableCompartment = useRef(new Compartment())
  const disabledRef = useRef(disabled)
  const onChangeRef = useRef(onChange)

  disabledRef.current = disabled

  useEffect(() => {
    onChangeRef.current = onChange
  }, [onChange])

  useEffect(() => {
    const container = containerRef.current
    if (container === null) return
    const editor = new EditorView({
      state: EditorState.create({
        doc: value,
        extensions: [
          markdown(),
          editableCompartment.current.of(
            EditorView.editable.of(!disabledRef.current),
          ),
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          syntaxHighlighting(defaultHighlightStyle),
          EditorView.lineWrapping,
          EditorView.contentAttributes.of({
            'aria-label': 'Markdown body',
            autocapitalize: 'sentences',
            spellcheck: 'true',
          }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged)
              onChangeRef.current(update.state.doc.toString())
          }),
        ],
      }),
      parent: container,
    })
    editorRef.current = editor
    return () => {
      editor.destroy()
      editorRef.current = null
    }
  }, [])

  useEffect(() => {
    editorRef.current?.dispatch({
      effects: editableCompartment.current.reconfigure(
        EditorView.editable.of(!disabled),
      ),
    })
  }, [disabled])

  useEffect(() => {
    const editor = editorRef.current
    if (editor === null || editor.state.doc.toString() === value) return
    editor.dispatch({
      changes: { from: 0, to: editor.state.doc.length, insert: value },
    })
  }, [value])

  return <div className="review-document-editor" ref={containerRef} />
}

export function ReviewDocumentPanel({
  summary,
  summaries = [],
  isMobile = false,
  canSave = true,
  canFinish = true,
  onSelectDocument = () => undefined,
  onOpenFocus = () => undefined,
  frontmatter,
  body,
  status = 'idle',
  unsavedChanges = false,
  error = null,
  view = 'preview',
  onFrontmatterChange = () => undefined,
  onBodyChange = () => undefined,
  onViewChange = () => undefined,
  onSave = () => undefined,
  onFinish = () => undefined,
}: ReviewDocumentPanelProps) {
  const busy = status === 'saving' || status === 'finishing'
  const updateFrontmatter = (
    path: readonly JsonPathPart[],
    value: JsonValue,
  ) => {
    onFrontmatterChange(updateAtPath(frontmatter, path, value))
  }
  const frontmatterEntries = isJsonObject(frontmatter)
    ? Object.entries(frontmatter).filter(
        ([name]) => !(summary?.kind === 'pr_draft' && name === 'title'),
      )
    : null
  const isIssueBody = summary?.kind === 'issue_body'
  const showFrontmatter =
    isIssueBody ||
    (summary?.kind !== 'review_replies' &&
      frontmatter !== null &&
      (frontmatterEntries === null || frontmatterEntries.length > 0))

  return (
    <article
      className="review-document-panel"
      data-view={view}
      data-status={status}
    >
      <ReviewDocumentNavigation
        {...(summary === undefined ? {} : { summary })}
        summaries={summaries}
        title={
          isJsonObject(frontmatter) && typeof frontmatter['title'] === 'string'
            ? frontmatter['title']
            : ''
        }
        canSave={canSave}
        busy={busy}
        onTitleChange={(title) => {
          if (isJsonObject(frontmatter)) {
            onFrontmatterChange({ ...frontmatter, title })
          }
        }}
        onSelectDocument={onSelectDocument}
      />
      <div className="review-document-content">
        {showFrontmatter ? (
          isIssueBody ? (
            <ReviewDocumentPreview
              mode="frontmatter"
              summary={summary}
              body={body}
              frontmatter={frontmatter}
              disabled={!canSave || busy}
              onFrontmatterChange={onFrontmatterChange}
            />
          ) : (
            <section
              className="review-document-frontmatter"
              aria-label="Document metadata"
            >
              <h2>Frontmatter</h2>
              {frontmatterEntries !== null ? (
                <dl className="review-document-fields">
                  {frontmatterEntries.map(([name, value]) => (
                    <FieldEditor
                      key={name}
                      name={name}
                      path={[name]}
                      value={value}
                      disabled={!canSave || busy}
                      onChange={updateFrontmatter}
                    />
                  ))}
                </dl>
              ) : (
                <FieldEditor
                  name="Frontmatter"
                  path={[]}
                  value={frontmatter}
                  disabled={!canSave || busy}
                  onChange={(path, value) => {
                    if (path.length === 0) onFrontmatterChange(value)
                  }}
                />
              )}
            </section>
          )
        ) : null}

        <section
          className="review-document-body-editor"
          aria-label="Document body"
        >
          <div className="review-document-body-heading">
            <h2>BODY</h2>
            <div className="review-document-body-toolbar">
              {isMobile ? null : (
                <button
                  aria-label="Open draft full width"
                  className="review-document-full-width"
                  type="button"
                  onClick={onOpenFocus}
                >
                  <Maximize2 aria-hidden="true" size={13} />
                  <span>Full width</span>
                </button>
              )}
              <div
                className="review-document-view-toggle"
                aria-label="Body view"
              >
                <button
                  aria-pressed={view === 'preview'}
                  type="button"
                  onClick={() => {
                    onViewChange('preview')
                  }}
                >
                  Preview
                </button>
                <button
                  aria-pressed={view === 'edit'}
                  type="button"
                  onClick={() => {
                    onViewChange('edit')
                  }}
                >
                  Edit
                </button>
              </div>
            </div>
          </div>
          <div className="review-document-body-columns">
            <div className="review-document-edit-column">
              <MarkdownEditor
                value={body}
                disabled={!canSave || busy}
                onChange={onBodyChange}
              />
            </div>
            <div className="review-document-preview-column">
              <ReviewDocumentPreview
                {...(summary === undefined ? {} : { summary })}
                body={body}
                canSave={canSave && !busy}
                onBodyChange={onBodyChange}
              />
            </div>
          </div>
        </section>
      </div>

      <footer className="review-document-actions">
        <span aria-live="polite" className="review-document-status">
          {status === 'saving'
            ? 'Saving…'
            : status === 'saved'
              ? 'Saved'
              : status === 'finishing'
                ? 'Finishing review…'
                : (error ?? (unsavedChanges ? 'Unsaved changes' : ''))}
        </span>
        {error === null ? null : (
          <span className="review-document-error" role="alert">
            {error}
          </span>
        )}
        <button disabled={!canSave || busy} type="button" onClick={onSave}>
          Save
        </button>
        <button
          className="review-document-finish"
          disabled={!canFinish || busy}
          type="button"
          onClick={onFinish}
        >
          Finish review
        </button>
      </footer>
    </article>
  )
}
