import type { FilterState, MessageSummary } from "./types";
import type { Theme } from "../stores/theme";
import type { Palette } from "../stores/palette";
import type { PreviewWidth } from "../stores/previewWidth";
import type { Density } from "../stores/density";
import { KEYS } from "./shortcuts";

/** The app state that decides which commands the palette offers. */
export interface CommandState {
  selected: MessageSummary | null;
  releaseConfigured: boolean;
  releasing: boolean;
  detailsShown: boolean;
  canUndo: boolean;
  total: number;
  filters: FilterState;
  tags: readonly string[];
  tagFiltersFull: boolean;
  theme: Theme;
  palette: Palette;
  palettes: readonly { id: Palette; label: string }[];
  rusted: boolean;
  density: Density;
  previewWidth: PreviewWidth;
  previewWidths: readonly { id: PreviewWidth; label: string }[];
}

type BooleanFilter = "starred" | "unread" | "attachments";

/** The store operations the commands run; the palette wires them up. */
export interface CommandActions {
  setStarred(id: string, starred: boolean): void;
  setRead(id: string, read: boolean): void;
  deleteSelected(): void;
  undoDelete(): void;
  downloadEml(id: string): void;
  release(message: MessageSummary): void;
  toggleDetails(): void;
  toggleFilter(filter: BooleanFilter): void;
  toggleTagFilter(tag: string): void;
  clearFilters(): void;
  clearInbox(): void;
  setTheme(theme: Theme): void;
  setPalette(palette: Palette): void;
  setRusted(on: boolean): void;
  toggleDensity(): void;
  setPreviewWidth(width: PreviewWidth): void;
  openSettings(): void;
  showShortcuts(): void;
}

/** One entry of the command palette. */
export interface Command {
  id: string;
  label: string;
  /** Extra words the query may match, never shown. */
  keywords?: string;
  shortcut?: string;
  run: () => void;
}

const THEMES: readonly { id: Theme; label: string }[] = [
  { id: "dark", label: "Dark" },
  { id: "light", label: "Light" },
  { id: "system", label: "System" },
];

const BOOLEAN_FILTERS: readonly { id: BooleanFilter; label: string }[] = [
  { id: "starred", label: "starred" },
  { id: "unread", label: "unread" },
  { id: "attachments", label: "attachments" },
];

function messageCommands(state: CommandState, actions: CommandActions) {
  const message = state.selected;
  if (!message) return [];
  const commands: Command[] = [
    {
      id: "message.star",
      label: message.is_starred ? "Unstar message" : "Star message",
      shortcut: KEYS.star,
      run: () => actions.setStarred(message.id, !message.is_starred),
    },
    {
      id: "message.read",
      label: message.is_read ? "Mark as unread" : "Mark as read",
      run: () => actions.setRead(message.id, !message.is_read),
    },
    {
      id: "message.delete",
      label: "Delete message",
      keywords: "remove trash",
      shortcut: KEYS.delete,
      run: actions.deleteSelected,
    },
    {
      id: "message.download",
      label: "Download .eml",
      keywords: "export save raw",
      run: () => actions.downloadEml(message.id),
    },
  ];
  if (state.releaseConfigured && !state.releasing) {
    commands.push({
      id: "message.release",
      label: "Release message",
      keywords: "send relay deliver",
      run: () => actions.release(message),
    });
  }
  commands.push({
    id: "message.details",
    label: state.detailsShown ? "Hide details" : "Show details",
    keywords: "rail drawer headers",
    shortcut: KEYS.details,
    run: actions.toggleDetails,
  });
  return commands;
}

function filterCommands(state: CommandState, actions: CommandActions) {
  const commands: Command[] = BOOLEAN_FILTERS.map((filter) => ({
    id: `filter.${filter.id}`,
    label: state.filters[filter.id]
      ? `Remove ${filter.label} filter`
      : `Filter by ${filter.label}`,
    run: () => actions.toggleFilter(filter.id),
  }));
  for (const tag of state.tags) {
    const active = state.filters.tags.includes(tag);
    if (!active && state.tagFiltersFull) continue;
    commands.push({
      id: `filter.tag.${tag}`,
      label: active ? `Remove tag filter: ${tag}` : `Filter by tag: ${tag}`,
      run: () => actions.toggleTagFilter(tag),
    });
  }
  const filtered =
    state.filters.starred ||
    state.filters.unread ||
    state.filters.attachments ||
    state.filters.tags.length > 0;
  if (filtered) {
    commands.push({
      id: "filter.clear",
      label: "Clear filters",
      shortcut: KEYS.escape,
      run: actions.clearFilters,
    });
  }
  return commands;
}

function appearanceCommands(state: CommandState, actions: CommandActions) {
  const commands: Command[] = [];
  for (const theme of THEMES) {
    if (theme.id === state.theme) continue;
    commands.push({
      id: `theme.${theme.id}`,
      label: `Theme: ${theme.label}`,
      keywords: "appearance mode",
      run: () => actions.setTheme(theme.id),
    });
  }
  for (const palette of state.palettes) {
    if (palette.id === state.palette) continue;
    commands.push({
      id: `palette.${palette.id}`,
      label: `Palette: ${palette.label}`,
      keywords: "appearance colors accent",
      run: () => actions.setPalette(palette.id),
    });
  }
  commands.push({
    id: "rusted",
    label: state.rusted ? "Turn Rusted off" : "Turn Rusted on",
    keywords: "appearance texture",
    run: () => actions.setRusted(!state.rusted),
  });
  commands.push({
    id: "density",
    label:
      state.density === "compact" ? "Density: Comfortable" : "Density: Compact",
    keywords: "appearance toggle density layout rows",
    run: actions.toggleDensity,
  });
  for (const width of state.previewWidths) {
    if (width.id === state.previewWidth) continue;
    commands.push({
      id: `preview.${width.id}`,
      label: `Preview width: ${width.label}`,
      keywords: "html responsive",
      run: () => actions.setPreviewWidth(width.id),
    });
  }
  return commands;
}

/**
 * Every command that does something in `state`, in palette order.
 *
 * A command that would do nothing right now, such as a message action with no
 * message selected or the theme already in use, is left out rather than shown
 * disabled.
 */
export function availableCommands(
  state: CommandState,
  actions: CommandActions,
): Command[] {
  const commands = [
    ...messageCommands(state, actions),
    ...filterCommands(state, actions),
  ];
  if (state.canUndo) {
    commands.push({
      id: "inbox.undo",
      label: "Undo delete",
      keywords: "restore",
      shortcut: KEYS.undo,
      run: actions.undoDelete,
    });
  }
  if (state.total > 0) {
    commands.push({
      id: "inbox.clear",
      label: "Clear inbox",
      keywords: "delete all",
      shortcut: KEYS.clearInbox,
      run: actions.clearInbox,
    });
  }
  return [
    ...commands,
    ...appearanceCommands(state, actions),
    {
      id: "settings",
      label: "Open settings",
      keywords: "preferences",
      run: actions.openSettings,
    },
    {
      id: "shortcuts",
      label: "Show keyboard shortcuts",
      keywords: "keys help",
      run: actions.showShortcuts,
    },
  ];
}
