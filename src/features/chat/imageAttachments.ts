// Composer-side image preparation. Vision tokens are pixels, so every image
// is downscaled BEFORE it leaves the machine: no edge past MAX_EDGE_PX and no
// more than MAX_TOTAL_PIXELS overall — the dual-cap rule the studio proved
// out. Oversized images are redrawn on a canvas and re-encoded as JPEG;
// images already inside the caps pass through untouched. GIFs always pass
// through (a canvas would freeze the animation) and rely on the size gate
// alone.

import type { PendingAttachment } from "./types";

const MAX_EDGE_PX = 1568;
const MAX_TOTAL_PIXELS = 1_150_000;
/** Files at or under this ride as-is when their pixels are inside the caps. */
const PASS_THROUGH_BYTES = 1_000_000;
/** Hard input gate — bigger than this is refused rather than processed. */
const MAX_INPUT_BYTES = 20_000_000;
const JPEG_QUALITY = 0.85;

const ACCEPTED_TYPES = new Set(["image/png", "image/jpeg", "image/webp", "image/gif"]);

export const MAX_ATTACHMENTS_PER_MESSAGE = 4;

export function isAttachableImage(file: File | Blob): boolean {
  return ACCEPTED_TYPES.has(file.type);
}

/** WebKitGTK often gives the paste event an EMPTY DataTransfer for a copied
 *  screenshot — the image is reachable only through the async clipboard API.
 *  Returns [] (never throws) when the clipboard holds no readable image. */
export async function readClipboardImages(): Promise<File[]> {
  try {
    const items = await navigator.clipboard.read();
    const files: File[] = [];
    for (const item of items) {
      const type = item.types.find((candidate) => ACCEPTED_TYPES.has(candidate));
      if (!type) continue;
      const blob = await item.getType(type);
      files.push(new File([blob], "clipboard-image", { type }));
    }
    return files;
  } catch {
    return [];
  }
}

/** Downscale (when needed) and encode one image for sending. Throws with a
 *  user-readable message when the file can't become an attachment. */
export async function prepareImageAttachment(file: File | Blob): Promise<PendingAttachment> {
  if (!isAttachableImage(file)) {
    throw new Error("Only PNG, JPEG, WebP and GIF images can be attached.");
  }
  if (file.size > MAX_INPUT_BYTES) {
    throw new Error("That image is too large to attach (over 20MB).");
  }
  // Animated GIFs would lose their animation on a canvas — send as-is.
  if (file.type === "image/gif") {
    if (file.size > PASS_THROUGH_BYTES * 5) {
      throw new Error("That GIF is too large to attach (over 5MB).");
    }
    return pendingFromBlob(file);
  }

  const image = await loadImage(file);
  try {
    const scale = fitScale(image.naturalWidth, image.naturalHeight);
    if (scale >= 1 && file.size <= PASS_THROUGH_BYTES) {
      return await pendingFromBlob(file);
    }
    const width = Math.max(1, Math.round(image.naturalWidth * Math.min(scale, 1)));
    const height = Math.max(1, Math.round(image.naturalHeight * Math.min(scale, 1)));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("The image could not be processed.");
    context.drawImage(image, 0, 0, width, height);
    const dataUrl = canvas.toDataURL("image/jpeg", JPEG_QUALITY);
    return pendingFromDataUrl(dataUrl);
  } finally {
    URL.revokeObjectURL(image.src);
  }
}

/** The factor that brings both caps into line; ≥ 1 means already inside. */
function fitScale(width: number, height: number): number {
  if (width <= 0 || height <= 0) return 1;
  const edgeScale = MAX_EDGE_PX / Math.max(width, height);
  const areaScale = Math.sqrt(MAX_TOTAL_PIXELS / (width * height));
  return Math.min(edgeScale, areaScale);
}

function loadImage(file: File | Blob): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("That file could not be read as an image."));
    };
    image.src = url;
  });
}

async function pendingFromBlob(file: File | Blob): Promise<PendingAttachment> {
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("That file could not be read."));
    reader.readAsDataURL(file);
  });
  return pendingFromDataUrl(dataUrl);
}

function pendingFromDataUrl(dataUrl: string): PendingAttachment {
  const match = /^data:([^;]+);base64,(.+)$/s.exec(dataUrl);
  if (!match) throw new Error("The image could not be encoded.");
  return { id: crypto.randomUUID(), mediaType: match[1], data: match[2] };
}
