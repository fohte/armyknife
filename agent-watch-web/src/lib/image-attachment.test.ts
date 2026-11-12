import { expect, it, vi } from 'vitest'

import { optimizeImageAttachment } from '#lib/image-attachment'

type TestDecodedImage = {
  width: number
  height: number
  draw: (
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
  ) => void
}

function decodedImage(width: number, height: number) {
  return { width, height, draw: () => undefined } satisfies TestDecodedImage
}

function imageBoundary(width: number, height: number, blobSize = 128) {
  const calls: {
    width: number
    height: number
    mimeType: string
    quality: number
  }[] = []
  return {
    calls,
    decode: vi.fn(() => Promise.resolve(decodedImage(width, height))),
    encode: vi.fn(
      (
        _image: TestDecodedImage,
        outputWidth: number,
        outputHeight: number,
        mimeType: string,
        quality: number,
      ) => {
        calls.push({
          width: outputWidth,
          height: outputHeight,
          mimeType,
          quality,
        })
        return Promise.resolve(
          new Blob([new Uint8Array(blobSize)], { type: mimeType }),
        )
      },
    ),
  }
}

function attachmentSnapshot(
  file: File,
  boundary: ReturnType<typeof imageBoundary>,
) {
  return {
    name: file.name,
    type: file.type,
    encodings: boundary.calls.map(({ width, height, mimeType }) => ({
      dimensions: [width, height],
      mimeType,
    })),
  }
}

function nonImageSnapshot(
  result: File,
  boundary: ReturnType<typeof imageBoundary>,
) {
  return { result, decodeCalls: boundary.decode.mock.calls }
}

function fileMetadata(file: File) {
  return { name: file.name, type: file.type }
}

it.each([
  {
    name: 'portrait',
    dimensions: [3000, 4000] as const,
    output: [1536, 2048] as const,
  },
  {
    name: 'landscape',
    dimensions: [4000, 3000] as const,
    output: [2048, 1536] as const,
  },
])(
  'converts an oversized $name image to a JPEG no larger than 2048 pixels',
  async ({ dimensions, output }) => {
    const file = new File(['source'], 'photo.heic', { type: 'image/heic' })
    const boundary = imageBoundary(dimensions[0], dimensions[1])

    const result = await optimizeImageAttachment(file, boundary)

    expect(attachmentSnapshot(result, boundary)).toEqual({
      name: 'photo.jpg',
      type: 'image/jpeg',
      encodings: [{ dimensions: output, mimeType: 'image/jpeg' }],
    })
  },
)

it('converts a decodable HEIC image before it reaches the attachment payload', async () => {
  const file = new File(['source'], 'photo.heic', { type: 'image/heic' })
  const boundary = imageBoundary(1200, 900)

  const result = await optimizeImageAttachment(file, boundary)

  expect(fileMetadata(result)).toEqual({
    name: 'photo.jpg',
    type: 'image/jpeg',
  })
})

it('keeps a small PNG unchanged to preserve its transparency', async () => {
  const file = new File(['small png'], 'screenshot.png', {
    type: 'image/png',
  })
  const boundary = imageBoundary(800, 600)

  const result = await optimizeImageAttachment(file, boundary)

  expect(result).toBe(file)
})

it('resizes an oversized PNG without removing its transparency', async () => {
  const file = new File(['large png'], 'screenshot.png', {
    type: 'image/png',
  })
  const boundary = imageBoundary(3000, 2000)

  const result = await optimizeImageAttachment(file, boundary)

  expect(attachmentSnapshot(result, boundary)).toEqual({
    name: 'screenshot.png',
    type: 'image/png',
    encodings: [{ dimensions: [2048, 1365], mimeType: 'image/png' }],
  })
})

it('keeps a bounded GIF animated by leaving its original bytes intact', async () => {
  const file = new File(['animated gif'], 'animation.gif', {
    type: 'image/gif',
  })
  const boundary = imageBoundary(640, 480)

  const result = await optimizeImageAttachment(file, boundary)

  expect(result).toBe(file)
})

it('keeps the original file when the browser cannot decode it', async () => {
  const file = new File(['unknown image'], 'photo.heic', {
    type: 'image/heic',
  })
  const boundary = {
    decode: vi.fn(() => Promise.reject(new Error('unsupported image'))),
    encode: vi.fn(),
  }

  const result = await optimizeImageAttachment(file, boundary)

  expect(result).toBe(file)
})

it('leaves non-image attachments untouched', async () => {
  const file = new File(['document'], 'notes.txt', { type: 'text/plain' })
  const boundary = imageBoundary(800, 600)

  const result = await optimizeImageAttachment(file, boundary)

  expect(nonImageSnapshot(result, boundary)).toEqual({
    result: file,
    decodeCalls: [],
  })
})
