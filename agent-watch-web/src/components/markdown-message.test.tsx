import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { MarkdownMessage } from '#components/markdown-message'

function renderMessage(
  markdown: string,
  className?: string,
  imageSources?: Readonly<Record<string, string | null>>,
  unmappedImagePathsAreLocal?: boolean,
) {
  const props = {
    ...(className === undefined ? {} : { className }),
    ...(imageSources === undefined ? {} : { imageSources }),
    ...(unmappedImagePathsAreLocal === undefined
      ? {}
      : { unmappedImagePathsAreLocal }),
  }
  return renderToStaticMarkup(
    <MarkdownMessage {...props}>{markdown}</MarkdownMessage>,
  )
}

it('renders common markdown and GFM as semantic elements', () => {
  const html = renderMessage(
    `## Summary

Use \`retry()\` with the [guide](https://example.com/guide).
First line
Second line

- Retry transient failures
- Stop after three attempts

| Attempt | Delay |
| --- | --- |
| 1 | 100 ms |

~~~rust
const maxPreviewItems = 7;
~~~`,
    'chat-user-message',
  )

  const expected = [
    '<div class="markdown-message chat-user-message"><h2>Summary</h2>',
    '<p>Use <code>retry()</code> with the <a href="https://example.com/guide" target="_blank" rel="noopener noreferrer">guide</a>.',
    'First line',
    'Second line</p>',
    '<ul>',
    '<li>Retry transient failures</li>',
    '<li>Stop after three attempts</li>',
    '</ul>',
    '<table><thead><tr><th>Attempt</th><th>Delay</th></tr></thead><tbody><tr><td>1</td><td>100 ms</td></tr></tbody></table>',
    '<div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">rust</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="rust"><code class="language-rust">const maxPreviewItems = 7;',
    '</code></pre></div></div>',
  ].join(String.fromCharCode(10))

  expect(html).toBe(expected)
})

it('renders markdown images with the shared conversation image treatment', () => {
  expect(
    renderMessage('![Sample screenshot](/api/images/fixture-image.png)'),
  ).toBe(
    '<div class="markdown-message"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge Sample screenshot"><img class="chat-user-image" src="/api/images/fixture-image.png" alt="Sample screenshot" loading="lazy" decoding="async"/></button></p></div>',
  )
})

it('shows unavailable for an unmapped local image by default', () => {
  expect(renderMessage('![](/Users/guest/screenshot.png)')).toBe(
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /Users/guest/screenshot.png</span></p></div>',
  )
})

it('keeps standalone root-relative image URLs', () => {
  expect(renderMessage('![Sample](/fixture-illustration.png)')).toBe(
    '<div class="markdown-message"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge Sample"><img class="chat-user-image" src="/fixture-illustration.png" alt="Sample" loading="lazy" decoding="async"/></button></p></div>',
  )
})

it('uses the server source for a copied assistant image', () => {
  expect(
    renderMessage('![](/tmp/fixture.png)', undefined, {
      '/tmp/fixture.png':
        '/api/sessions/fixture-session/answer-images/fixture-key',
    }),
  ).toBe(
    '<div class="markdown-message"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge image"><img class="chat-user-image" src="/api/sessions/fixture-session/answer-images/fixture-key" alt="" loading="lazy" decoding="async"/></button></p></div>',
  )
})

it('matches copied assistant images with spaces and non-ASCII path characters', () => {
  const path = '/tmp/画像 samples/final image.png'
  expect(
    renderMessage(`![](<${path}>)`, undefined, {
      [path]: '/api/sessions/fixture-session/answer-images/fixture-key',
    }),
  ).toBe(
    '<div class="markdown-message"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge image"><img class="chat-user-image" src="/api/sessions/fixture-session/answer-images/fixture-key" alt="" loading="lazy" decoding="async"/></button></p></div>',
  )
})

const escapedPath = String.raw`/tmp/fixture\(v1\).png`

it.each([
  [
    'a Markdown escape is not mapped',
    `![](<${escapedPath}>)`,
    '/tmp/fixture(v1).png',
  ],
  [
    'a reference image is not mapped',
    '![fixture][sample-image]\n\n[sample-image]: </tmp/fixture.png>',
    '/tmp/fixture.png',
  ],
])('shows unavailable local images when %s', (_name, markdown, src) => {
  expect(renderMessage(markdown, undefined, {}, true)).toBe(
    `<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: ${src}</span></p></div>`,
  )
})

it('shows an unavailable message for an ineligible assistant image', () => {
  expect(
    renderMessage('![](/tmp/fixture.txt)', undefined, {
      '/tmp/fixture.txt': null,
    }),
  ).toBe(
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/fixture.txt</span></p></div>',
  )
})

it.each([
  [
    'a matched unavailable path',
    { '/tmp/画像 samples/final image.png': null },
    '/tmp/画像 samples/final image.png',
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/画像 samples/final image.png</span></p></div>',
  ],
  [
    'an unmatched path',
    {},
    '/tmp/画像 samples/final image.png',
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/画像 samples/final image.png</span></p></div>',
  ],
  [
    'a matched path containing a literal percent sequence',
    { '/tmp/fixture%20name.png': null },
    '/tmp/fixture%20name.png',
    '<div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/fixture%20name.png</span></p></div>',
  ],
])('shows the original path for %s', (_name, imageSources, path, expected) => {
  expect(renderMessage(`![](<${path}>)`, undefined, imageSources, true)).toBe(
    expected,
  )
})

it('keeps untagged fenced code in a preformatted block', () => {
  const html = renderMessage(`~~~
set VALUE=preview
~~~`)

  expect(html).toBe(
    '<div class="markdown-message"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">set VALUE=preview\n</code></pre></div></div>',
  )
})

it('shows a copy toolbar for every fenced code block', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage>{'~~~text\nsample\n~~~'}</MarkdownMessage>,
  )

  expect(html).toBe(
    '<div class="markdown-message"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">sample\n</code></pre></div></div>',
  )
})

it('keeps raw HTML as text and blocks unsafe link protocols', () => {
  const html = renderMessage(
    '<img src=x onerror=alert(1)> [unsafe](javascript:alert(1))',
  )

  expect(html).toBe(
    '<div class="markdown-message"><p>&lt;img src=x onerror=alert(1)&gt; <a href="" target="_blank" rel="noopener noreferrer">unsafe</a></p></div>',
  )
})

it('renders br tags as line breaks in paragraphs and table cells', () => {
  const html = renderMessage(
    `before<br>middle<BR/>after<br />last

| Content |
| --- |
| first<br>second<BR/>third<br />fourth |`,
  )

  expect(html).toBe(
    [
      '<div class="markdown-message"><p>before<br/>',
      'middle<br/>',
      'after<br/>',
      'last</p>',
      '<table><thead><tr><th>Content</th></tr></thead><tbody><tr><td>first<br/>',
      'second<br/>',
      'third<br/>',
      'fourth</td></tr></tbody></table></div>',
    ].join(String.fromCharCode(10)),
  )
})

it('trims non-ASCII text from automatic links and preserves explicit links', () => {
  const html = renderMessage(
    'http://localhost:43127。承認後に処理する [http://localhost:43127。承認後に処理する](http://localhost:43127。承認後に処理する) and http://example.com and http://example.com/日本語',
  )

  expect(html).toBe(
    '<div class="markdown-message"><p><a href="http://localhost:43127" target="_blank" rel="noopener noreferrer">http://localhost:43127</a>。承認後に処理する <a href="http://localhost:43127%E3%80%82%E6%89%BF%E8%AA%8D%E5%BE%8C%E3%81%AB%E5%87%A6%E7%90%86%E3%81%99%E3%82%8B" target="_blank" rel="noopener noreferrer">http://localhost:43127。承認後に処理する</a> and <a href="http://example.com" target="_blank" rel="noopener noreferrer">http://example.com</a> and <a href="http://example.com/" target="_blank" rel="noopener noreferrer">http://example.com/</a>日本語</p></div>',
  )
})

it('adds a confirmation button to agent inline bang commands', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage sessionId="sample-session">
      {'Run `! printf safe`.'}
    </MarkdownMessage>,
  )

  expect(html).toBe(
    '<div class="markdown-message"><p>Run <span class="bang-command-inline"><code>! printf safe</code><span class="bang-command-actions"><span class="bang-command-control"><button aria-label="Run command 1" class="bang-command-button" type="button">Run command 1</button></span></span></span>.</p></div>',
  )
})

it('adds per-line and run-all buttons to fenced bang commands', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage sessionId="sample-session">
      {'```sh\n! printf first\n! printf second\n```'}
    </MarkdownMessage>,
  )

  expect(html).toBe(
    [
      '<div class="markdown-message"><div class="markdown-code-block"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">sh</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="sh"><code class="language-sh">! printf first',
      '! printf second',
      '</code></pre></div><span class="bang-command-actions"><span class="bang-command-control"><button aria-label="Run command 1" class="bang-command-button" type="button">Run command 1</button></span><span class="bang-command-control"><button aria-label="Run command 2" class="bang-command-button" type="button">Run command 2</button></span><span class="bang-command-control"><button aria-label="Run all" class="bang-command-button" type="button">Run all</button></span></span></div></div>',
    ].join(String.fromCharCode(10)),
  )
})

it('adds a confirmation button to an indented bang command block', () => {
  const html = renderToStaticMarkup(
    <MarkdownMessage sessionId="sample-session">
      {'    ! printf safe'}
    </MarkdownMessage>,
  )

  expect(html).toBe(
    '<div class="markdown-message"><div class="markdown-code-block"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">text</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="text"><code class="language-text">! printf safe\n</code></pre></div><span class="bang-command-actions"><span class="bang-command-control"><button aria-label="Run command 1" class="bang-command-button" type="button">Run command 1</button></span></span></div></div>',
  )
})
