import { createSignal } from "solid-js";

/** Every width the HTML preview renders at, in switch order. */
const PREVIEW_WIDTHS = [
  { id: "desktop", label: "Desktop" },
  { id: "mobile", label: "Mobile" },
] as const;

type PreviewWidth = (typeof PREVIEW_WIDTHS)[number]["id"];

const STORAGE_KEY = "rustmail-preview-width";
const DEFAULT_PREVIEW_WIDTH: PreviewWidth = "desktop";

/** The viewport width a phone renders the email at, in CSS pixels. */
const MOBILE_PREVIEW_WIDTH_PX = 375;

function isPreviewWidth(value: string | null): value is PreviewWidth {
  return PREVIEW_WIDTHS.some((width) => width.id === value);
}

function getInitialPreviewWidth(): PreviewWidth {
  const stored = localStorage.getItem(STORAGE_KEY);
  return isPreviewWidth(stored) ? stored : DEFAULT_PREVIEW_WIDTH;
}

const [previewWidth, setPreviewWidthSignal] = createSignal<PreviewWidth>(
  getInitialPreviewWidth(),
);

function setPreviewWidth(width: PreviewWidth) {
  setPreviewWidthSignal(width);
  localStorage.setItem(STORAGE_KEY, width);
}

export {
  PREVIEW_WIDTHS,
  MOBILE_PREVIEW_WIDTH_PX,
  previewWidth,
  setPreviewWidth,
};
export type { PreviewWidth };
