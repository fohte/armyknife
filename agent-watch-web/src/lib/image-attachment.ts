import { ResultAsync } from 'neverthrow'

const MAX_IMAGE_DIMENSION = 2048
const JPEG_QUALITY = 0.9
const SUPPORTED_IMAGE_MIME_TYPES = new Set([
  'image/gif',
  'image/jpeg',
  'image/png',
  'image/webp',
])
const PNG_PRESERVING_MIME_TYPES = new Set([
  'image/gif',
  'image/png',
  'image/webp',
])
const IMAGE_EXTENSIONS = new Set([
  'avif',
  'gif',
  'heic',
  'heif',
  'jpeg',
  'jpg',
  'png',
  'webp',
])

type DecodedImage = {
  width: number
  height: number
  draw: (
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
  ) => void
}

type ImageAttachmentBoundary = {
  decode: (file: File) => Promise<DecodedImage>
  encode: (
    image: DecodedImage,
    width: number,
    height: number,
    mimeType: string,
    quality: number,
  ) => Promise<Blob | null>
}

const browserBoundary: ImageAttachmentBoundary = {
  decode: (file) =>
    new Promise((resolve, reject) => {
      const url = URL.createObjectURL(file)
      const image = new Image()
      const finish = (result: DecodedImage | Error) => {
        URL.revokeObjectURL(url)
        if (result instanceof Error) {
          reject(result)
        } else {
          resolve(result)
        }
      }

      image.onload = () => {
        if (image.naturalWidth === 0 || image.naturalHeight === 0) {
          finish(new Error('Image has no dimensions'))
          return
        }
        finish({
          width: image.naturalWidth,
          height: image.naturalHeight,
          draw: (context, width, height) => {
            context.drawImage(image, 0, 0, width, height)
          },
        })
      }
      image.onerror = () => {
        finish(new Error('Image could not be decoded'))
      }
      image.src = url
    }),
  encode: (image, width, height, mimeType, quality) => {
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const context = canvas.getContext('2d')
    if (context === null) return Promise.resolve(null)
    image.draw(context, width, height)
    return new Promise((resolve) => {
      canvas.toBlob(resolve, mimeType, quality)
    })
  },
}

function isImageFile(file: File) {
  if (file.type.toLowerCase().startsWith('image/')) return true
  const extension = file.name.split('.').at(-1)?.toLowerCase()
  return extension !== undefined && IMAGE_EXTENSIONS.has(extension)
}

function imageFilename(name: string, mimeType: string) {
  const stem = name.replace(/\.[^.]*$/, '') || 'Pasted image'
  return `${stem}.${mimeType === 'image/jpeg' ? 'jpg' : 'png'}`
}

export async function optimizeImageAttachment(
  file: File,
  boundary: ImageAttachmentBoundary = browserBoundary,
): Promise<File> {
  if (!isImageFile(file)) return file

  return ResultAsync.fromPromise(
    optimizeDecodedImage(file, boundary),
    (error) => (error instanceof Error ? error : new Error(String(error))),
  ).match(
    (optimizedFile) => optimizedFile,
    () => file,
  )
}

async function optimizeDecodedImage(
  file: File,
  boundary: ImageAttachmentBoundary,
) {
  const image = await boundary.decode(file)
  if (
    !Number.isFinite(image.width) ||
    !Number.isFinite(image.height) ||
    image.width <= 0 ||
    image.height <= 0
  ) {
    return file
  }

  const scale = Math.min(
    1,
    MAX_IMAGE_DIMENSION / Math.max(image.width, image.height),
  )
  const width = Math.max(1, Math.round(image.width * scale))
  const height = Math.max(1, Math.round(image.height * scale))
  const needsResize = width !== image.width || height !== image.height
  const sourceMimeType = file.type.toLowerCase()
  if (!needsResize && SUPPORTED_IMAGE_MIME_TYPES.has(sourceMimeType))
    return file

  const outputMimeType = PNG_PRESERVING_MIME_TYPES.has(sourceMimeType)
    ? 'image/png'
    : 'image/jpeg'
  const blob = await boundary.encode(
    image,
    width,
    height,
    outputMimeType,
    JPEG_QUALITY,
  )
  if (blob === null || blob.type !== outputMimeType) return file

  return new File([blob], imageFilename(file.name, outputMimeType), {
    type: outputMimeType,
    lastModified: file.lastModified,
  })
}
