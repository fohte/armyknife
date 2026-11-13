import '#components/review-document-panel.css'

import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { markdown } from '@codemirror/lang-markdown'
import { defaultHighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { Compartment, EditorState } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { useEffect, useRef, useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import type { JsonValue } from '#types'

type ReviewDocumentPanelProps = {
  frontmatter: JsonValue
  body: string
  status?: 'idle' | 'saving' | 'saved' | 'error' | 'finishing'
  error?: string | null
  mobileView?: 'edit' | 'preview'
  onFrontmatterChange?: (value: JsonValue) => void
  onBodyChange?: (value: string) => void
  onMobileViewChange?: (view: 'edit' | 'preview') => void
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
  frontmatter,
  body,
  status = 'idle',
  error = null,
  mobileView = 'edit',
  onFrontmatterChange = () => undefined,
  onBodyChange = () => undefined,
  onMobileViewChange = () => undefined,
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

  return (
    <article
      className="review-document-panel"
      data-mobile-view={mobileView}
      data-status={status}
    >
      <section
        className="review-document-frontmatter"
        aria-label="Document metadata"
      >
        <h2>Frontmatter</h2>
        {isJsonObject(frontmatter) ? (
          <dl className="review-document-fields">
            {Object.entries(frontmatter).map(([name, value]) => (
              <FieldEditor
                key={name}
                name={name}
                path={[name]}
                value={value}
                disabled={busy}
                onChange={updateFrontmatter}
              />
            ))}
          </dl>
        ) : (
          <FieldEditor
            name="Frontmatter"
            path={[]}
            value={frontmatter}
            disabled={busy}
            onChange={(path, value) => {
              if (path.length === 0) onFrontmatterChange(value)
            }}
          />
        )}
      </section>

      <section
        className="review-document-body-editor"
        aria-label="Document body"
      >
        <div className="review-document-body-heading">
          <h2>Body</h2>
          <div className="review-document-view-toggle" aria-label="Body view">
            <button
              aria-pressed={mobileView === 'edit'}
              type="button"
              onClick={() => {
                onMobileViewChange('edit')
              }}
            >
              Edit
            </button>
            <button
              aria-pressed={mobileView === 'preview'}
              type="button"
              onClick={() => {
                onMobileViewChange('preview')
              }}
            >
              Preview
            </button>
          </div>
        </div>
        <div className="review-document-body-columns">
          <div className="review-document-edit-column">
            <MarkdownEditor
              value={body}
              disabled={busy}
              onChange={onBodyChange}
            />
          </div>
          <div className="review-document-preview-column">
            <MarkdownMessage className="review-document-body">
              {body}
            </MarkdownMessage>
          </div>
        </div>
      </section>

      <footer className="review-document-actions">
        <span aria-live="polite" className="review-document-status">
          {status === 'saving'
            ? 'Saving…'
            : status === 'saved'
              ? 'Saved'
              : status === 'finishing'
                ? 'Finishing review…'
                : error}
        </span>
        {error === null ? null : (
          <span className="review-document-error" role="alert">
            {error}
          </span>
        )}
        <button disabled={busy} type="button" onClick={onSave}>
          Save
        </button>
        <button
          className="review-document-finish"
          disabled={busy}
          type="button"
          onClick={onFinish}
        >
          Finish review
        </button>
      </footer>
    </article>
  )
}
