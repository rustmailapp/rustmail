/** The UI's icons, outline unless named otherwise. */

const STAR_PATH =
  "M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.562.562 0 00-.182-.557l-4.204-3.602a.563.563 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z";
const PAPERCLIP_PATH =
  "M18.375 12.739l-7.693 7.693a4.5 4.5 0 01-6.364-6.364l10.94-10.94A3 3 0 1119.5 7.372L8.552 18.32m.009-.01l-.01.01m5.699-9.941l-7.81 7.81a1.5 1.5 0 002.112 2.13";
const DOWNLOAD_PATH =
  "M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5M16.5 12L12 16.5m0 0L7.5 12m4.5 4.5V3";
const TRASH_PATH =
  "M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0";
const COPY_PATH =
  "M15.75 17.25v3.375c0 .621-.504 1.125-1.125 1.125h-9.75a1.125 1.125 0 01-1.125-1.125V7.875c0-.621.504-1.125 1.125-1.125H6.75a9.06 9.06 0 011.5.124m7.5 10.376h3.375c.621 0 1.125-.504 1.125-1.125V11.25c0-4.46-3.243-8.161-7.5-8.876a9.06 9.06 0 00-1.5-.124H9.375c-.621 0-1.125.504-1.125 1.125v3.5m7.5 10.375H9.375a1.125 1.125 0 01-1.125-1.125v-9.25m12 6.625v-1.875a3.375 3.375 0 00-3.375-3.375h-1.5a1.125 1.125 0 01-1.125-1.125v-1.5a3.375 3.375 0 00-3.375-3.375H9.75";
const PANEL_RIGHT_PATH =
  "M5 3h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2zM15 3v18";
const INFO_PATH =
  "M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9-3.75h.008v.008H12V8.25z";
const SEND_PATH =
  "M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5";
const TAG_PATH =
  "M9.568 3H5.25A2.25 2.25 0 003 5.25v4.318c0 .597.237 1.17.659 1.591l9.581 9.581c.699.699 1.78.872 2.607.33a18.095 18.095 0 005.223-5.223c.542-.827.369-1.908-.33-2.607L11.16 3.66A2.25 2.25 0 009.568 3z M6 6h.008v.008H6V6z";
const GEAR_PATH =
  "M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 010 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 010-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28z M15 12a3 3 0 11-6 0 3 3 0 016 0z";
const SEARCH_PATH =
  "M21 21l-5.197-5.197m0 0A7.5 7.5 0 105.196 5.196a7.5 7.5 0 0010.607 10.607z";
const CHEVRON_DOWN_PATH = "M19.5 8.25l-7.5 7.5-7.5-7.5";
const CHECK_PATH = "M4.5 12.75l6 6 9-13.5";
const FUNNEL_PATH =
  "M12 3c2.755 0 5.455.232 8.083.678.533.09.917.556.917 1.096v1.044a2.25 2.25 0 01-.659 1.591l-5.432 5.432a2.25 2.25 0 00-.659 1.591v2.927a2.25 2.25 0 01-1.244 2.013L9.75 21v-6.568a2.25 2.25 0 00-.659-1.591L3.659 7.409A2.25 2.25 0 013 5.818V4.774c0-.54.384-1.006.917-1.096A48.32 48.32 0 0112 3z";
const CLOSE_PATH = "M6 18L18 6M6 6l12 12";
const EYE_PATH =
  "M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.64 0 8.577 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.64 0-8.577-3.007-9.963-7.178z M15 12a3 3 0 11-6 0 3 3 0 016 0z";
const ENVELOPE_PATH =
  "M21.75 6.75v10.5a2.25 2.25 0 01-2.25 2.25h-15a2.25 2.25 0 01-2.25-2.25V6.75m19.5 0A2.25 2.25 0 0019.5 4.5h-15a2.25 2.25 0 00-2.25 2.25m19.5 0v.243a2.25 2.25 0 01-1.07 1.916l-7.5 4.615a2.25 2.25 0 01-2.36 0L3.32 8.91a2.25 2.25 0 01-1.07-1.916V6.75";
const MOON_PATH =
  "M21.752 15.002A9.718 9.718 0 0118 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 003 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 009.002-5.998z";
const SUN_PATH =
  "M12 3v2.25m6.364.386l-1.591 1.591M21 12h-2.25m-.386 6.364l-1.591-1.591M12 18.75V21m-4.773-4.227l-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0z";
const MONITOR_PATH =
  "M9 17.25v1.007a3 3 0 01-.879 2.122L7.5 21h9l-.621-.621A3 3 0 0115 18.257V17.25m6-12V15a2.25 2.25 0 01-2.25 2.25H5.25A2.25 2.25 0 013 15V5.25A2.25 2.25 0 015.25 3h13.5A2.25 2.25 0 0121 5.25z";
const PAINTBRUSH_PATH =
  "M4.098 19.902a3.75 3.75 0 005.304 0l6.401-6.402M6.75 21A3.75 3.75 0 013 17.25V4.125C3 3.504 3.504 3 4.125 3h5.25c.621 0 1.125.504 1.125 1.125v4.072M6.75 21a3.75 3.75 0 003.75-3.75V8.197M6.75 21h13.125c.621 0 1.125-.504 1.125-1.125v-5.25c0-.621-.504-1.125-1.125-1.125h-4.072M10.5 8.197l2.88-2.88c.438-.439 1.15-.439 1.59 0l3.712 3.713c.44.44.44 1.152 0 1.59l-2.879 2.88M6.75 17.25h.008v.008H6.75v-.008z";
const BARS_PATH = "M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5";

function Outline(props: {
  path: string;
  class: string;
  filled?: boolean;
  strokeWidth?: number;
}) {
  return (
    <svg
      class={props.class}
      viewBox="0 0 24 24"
      fill={props.filled ? "currentColor" : "none"}
      stroke="currentColor"
      stroke-width={props.strokeWidth ?? 2}
      aria-hidden="true"
    >
      <path stroke-linecap="round" stroke-linejoin="round" d={props.path} />
    </svg>
  );
}

export function StarIcon(props: { class: string; filled: boolean }) {
  return <Outline path={STAR_PATH} class={props.class} filled={props.filled} />;
}

export function PaperclipIcon(props: { class: string }) {
  return <Outline path={PAPERCLIP_PATH} class={props.class} />;
}

export function TagIcon(props: { class: string }) {
  return <Outline path={TAG_PATH} class={props.class} />;
}

export function DownloadIcon(props: { class: string }) {
  return <Outline path={DOWNLOAD_PATH} class={props.class} />;
}

export function TrashIcon(props: { class: string }) {
  return <Outline path={TRASH_PATH} class={props.class} />;
}

export function CopyIcon(props: { class: string }) {
  return <Outline path={COPY_PATH} class={props.class} />;
}

export function InfoIcon(props: { class: string }) {
  return <Outline path={INFO_PATH} class={props.class} />;
}

export function GearIcon(props: { class: string }) {
  return <Outline path={GEAR_PATH} class={props.class} />;
}

export function PanelRightIcon(props: { class: string }) {
  return <Outline path={PANEL_RIGHT_PATH} class={props.class} />;
}

export function SendIcon(props: { class: string }) {
  return <Outline path={SEND_PATH} class={props.class} />;
}

export function SearchIcon(props: { class: string }) {
  return <Outline path={SEARCH_PATH} class={props.class} />;
}

export function ChevronDownIcon(props: { class: string }) {
  return <Outline path={CHEVRON_DOWN_PATH} class={props.class} />;
}

export function CheckIcon(props: { class: string }) {
  return <Outline path={CHECK_PATH} class={props.class} strokeWidth={3} />;
}

export function CloseIcon(props: { class: string; strokeWidth?: number }) {
  return (
    <Outline
      path={CLOSE_PATH}
      class={props.class}
      strokeWidth={props.strokeWidth}
    />
  );
}

export function MoonIcon(props: { class: string }) {
  return <Outline path={MOON_PATH} class={props.class} />;
}

export function SunIcon(props: { class: string }) {
  return <Outline path={SUN_PATH} class={props.class} />;
}

export function MonitorIcon(props: { class: string }) {
  return <Outline path={MONITOR_PATH} class={props.class} />;
}

export function PaintBrushIcon(props: { class: string }) {
  return <Outline path={PAINTBRUSH_PATH} class={props.class} />;
}

export function BarsIcon(props: { class: string }) {
  return <Outline path={BARS_PATH} class={props.class} />;
}

/** Empty-state art: drawn large, so with a lighter stroke. */
export function FunnelIcon(props: { class: string }) {
  return <Outline path={FUNNEL_PATH} class={props.class} strokeWidth={1.5} />;
}

/** Empty-state art: drawn large, so with a lighter stroke. */
export function EyeIcon(props: { class: string }) {
  return <Outline path={EYE_PATH} class={props.class} strokeWidth={1.5} />;
}

/** Empty-state art: drawn large, so with a lighter stroke. */
export function EnvelopeIcon(props: { class: string }) {
  return <Outline path={ENVELOPE_PATH} class={props.class} strokeWidth={1.5} />;
}

/** A small solid cross, for removing an item from a list. */
export function XMarkMiniIcon(props: { class: string }) {
  return (
    <svg
      class={props.class}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M6.28 5.22a.75.75 0 00-1.06 1.06L8.94 10l-3.72 3.72a.75.75 0 101.06 1.06L10 11.06l3.72 3.72a.75.75 0 101.06-1.06L11.06 10l3.72-3.72a.75.75 0 00-1.06-1.06L10 8.94 6.28 5.22z" />
    </svg>
  );
}
