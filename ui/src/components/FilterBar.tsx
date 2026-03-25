import { For, Show, createSignal, onCleanup, createEffect, on } from "solid-js";
import { Portal } from "solid-js/web";
import {
  search,
  setSearch,
  fetchMessages,
  filters,
  hasActiveFilters,
  clearFilters,
  clearTagFilters,
  toggleFilter,
  toggleTagFilter,
  allTags,
  filteredMessages,
  total,
} from "../stores/messages";

function Chip(props: {
  label: string;
  active: boolean;
  activeClass: string;
  onClick: () => void;
  icon?: string;
}) {
  return (
    <button
      onClick={props.onClick}
      class={`inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-[11px] font-medium border transition-colors cursor-pointer select-none ${
        props.active
          ? props.activeClass
          : "bg-zinc-100 dark:bg-zinc-800/50 text-zinc-500 dark:text-zinc-500 border-zinc-200 dark:border-zinc-700/50 hover:bg-zinc-200 dark:hover:bg-zinc-700/50 hover:text-zinc-600 dark:hover:text-zinc-400"
      }`}
    >
      <Show when={props.icon}>
        <span class="text-[10px]">{props.icon}</span>
      </Show>
      {props.label}
    </button>
  );
}

function TagDropdown() {
  const [open, setOpen] = createSignal(false);
  const [pos, setPos] = createSignal({ top: 0, left: 0 });
  let triggerRef: HTMLButtonElement | undefined;
  let dropdownRef: HTMLDivElement | undefined;

  function updatePosition() {
    if (!triggerRef) return;
    const rect = triggerRef.getBoundingClientRect();
    setPos({ top: rect.bottom + 4, left: rect.left });
  }

  function handleClickOutside(e: MouseEvent) {
    if (
      dropdownRef &&
      !dropdownRef.contains(e.target as Node) &&
      triggerRef &&
      !triggerRef.contains(e.target as Node)
    ) {
      setOpen(false);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && open()) {
      e.stopPropagation();
      setOpen(false);
    }
  }

  createEffect(
    on(
      open,
      (isOpen) => {
        if (isOpen) {
          updatePosition();
          window.addEventListener("scroll", updatePosition, true);
          window.addEventListener("resize", updatePosition);
          document.addEventListener("mousedown", handleClickOutside);
          document.addEventListener("keydown", handleKeydown, true);
        }

        onCleanup(() => {
          window.removeEventListener("scroll", updatePosition, true);
          window.removeEventListener("resize", updatePosition);
          document.removeEventListener("mousedown", handleClickOutside);
          document.removeEventListener("keydown", handleKeydown, true);
        });
      },
      { defer: true },
    ),
  );

  return (
    <>
      <button
        ref={triggerRef}
        onClick={() => setOpen(!open())}
        class={`inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-[11px] font-medium border transition-colors cursor-pointer select-none ${
          filters().tags.length > 0
            ? "bg-orange-50 dark:bg-orange-500/15 text-orange-600 dark:text-orange-400 border-orange-200 dark:border-orange-500/30"
            : "bg-zinc-100 dark:bg-zinc-800/50 text-zinc-500 dark:text-zinc-500 border-zinc-200 dark:border-zinc-700/50 hover:bg-zinc-200 dark:hover:bg-zinc-700/50 hover:text-zinc-600 dark:hover:text-zinc-400"
        }`}
      >
        <svg
          class="size-3"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            d="M9.568 3H5.25A2.25 2.25 0 003 5.25v4.318c0 .597.237 1.17.659 1.591l9.581 9.581c.699.699 1.78.872 2.607.33a18.095 18.095 0 005.223-5.223c.542-.827.369-1.908-.33-2.607L11.16 3.66A2.25 2.25 0 009.568 3z"
          />
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            d="M6 6h.008v.008H6V6z"
          />
        </svg>
        Tags
        <Show when={filters().tags.length > 0}>
          <span class="bg-orange-200 dark:bg-orange-500/30 text-orange-700 dark:text-orange-300 rounded-full px-1 text-[10px] leading-tight">
            {filters().tags.length}
          </span>
        </Show>
        <svg
          class={`size-3 transition-transform ${open() ? "rotate-180" : ""}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            d="M19.5 8.25l-7.5 7.5-7.5-7.5"
          />
        </svg>
      </button>

      <Show when={open()}>
        <Portal>
          <div
            ref={dropdownRef}
            class="fixed z-50 min-w-[180px] max-h-[240px] overflow-y-auto rounded-lg border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 shadow-lg"
            style={{ top: `${pos().top}px`, left: `${pos().left}px` }}
          >
            <Show
              when={allTags().length > 0}
              fallback={
                <div class="px-3 py-4 text-xs text-zinc-400 dark:text-zinc-500 text-center">
                  No tags yet
                </div>
              }
            >
              <div class="py-1">
                <For each={allTags()}>
                  {(tag) => {
                    const isSelected = () => filters().tags.includes(tag);
                    return (
                      <button
                        onClick={() => toggleTagFilter(tag)}
                        class="flex items-center gap-2 w-full px-3 py-1.5 text-left text-xs hover:bg-zinc-50 dark:hover:bg-zinc-800 transition-colors cursor-pointer"
                      >
                        <div
                          class={`size-3.5 rounded border flex items-center justify-center transition-colors ${
                            isSelected()
                              ? "bg-orange-500 border-orange-500"
                              : "border-zinc-300 dark:border-zinc-600"
                          }`}
                        >
                          <Show when={isSelected()}>
                            <svg
                              class="size-2.5 text-white"
                              viewBox="0 0 24 24"
                              fill="none"
                              stroke="currentColor"
                              stroke-width="3"
                            >
                              <path
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                d="M4.5 12.75l6 6 9-13.5"
                              />
                            </svg>
                          </Show>
                        </div>
                        <span
                          class={
                            isSelected()
                              ? "text-zinc-900 dark:text-zinc-100"
                              : "text-zinc-600 dark:text-zinc-400"
                          }
                        >
                          {tag}
                        </span>
                      </button>
                    );
                  }}
                </For>
              </div>
              <Show when={filters().tags.length > 0}>
                <div class="border-t border-zinc-100 dark:border-zinc-800 px-3 py-1.5">
                  <button
                    onClick={() => {
                      clearTagFilters();
                      setOpen(false);
                    }}
                    class="text-[11px] text-zinc-400 dark:text-zinc-500 hover:text-zinc-600 dark:hover:text-zinc-300 transition cursor-pointer"
                  >
                    Reset tags
                  </button>
                </div>
              </Show>
            </Show>
          </div>
        </Portal>
      </Show>
    </>
  );
}

export default function FilterBar() {
  const f = filters;
  let debounceTimer: ReturnType<typeof setTimeout>;

  function onSearchInput(value: string) {
    setSearch(value);
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => fetchMessages(), 250);
  }

  return (
    <div class="flex-shrink-0 border-b border-zinc-200 dark:border-zinc-800/50">
      <div class="px-3 pt-2.5 pb-2">
        <input
          type="text"
          placeholder="Search emails..."
          value={search()}
          onInput={(e) => onSearchInput(e.currentTarget.value)}
          class="w-full rounded-lg border border-zinc-300 dark:border-zinc-800 bg-zinc-100 dark:bg-zinc-900 px-3 py-1.5 text-sm text-zinc-800 dark:text-zinc-200 placeholder-zinc-500 dark:placeholder-zinc-500 outline-none focus:border-zinc-400 dark:focus:border-zinc-600 transition"
        />
      </div>

      <div class="flex flex-wrap items-center gap-1.5 px-3 pb-2">
        <Chip
          label="Starred"
          icon={"\u2605"}
          active={f().starred}
          activeClass="bg-amber-50 dark:bg-amber-500/15 text-amber-600 dark:text-amber-400 border-amber-200 dark:border-amber-500/30"
          onClick={() => toggleFilter("starred")}
        />
        <Chip
          label="Unread"
          active={f().unread}
          activeClass="bg-blue-50 dark:bg-blue-500/15 text-blue-600 dark:text-blue-400 border-blue-200 dark:border-blue-500/30"
          onClick={() => toggleFilter("unread")}
        />
        <Chip
          label="Attachments"
          active={f().attachments}
          activeClass="bg-violet-50 dark:bg-violet-500/15 text-violet-600 dark:text-violet-400 border-violet-200 dark:border-violet-500/30"
          onClick={() => toggleFilter("attachments")}
        />

        <TagDropdown />

        <Show when={hasActiveFilters()}>
          <div class="ml-auto flex items-center gap-2">
            <span class="text-[11px] text-zinc-400 dark:text-zinc-600">
              {filteredMessages().length}/{total()}
            </span>
            <button
              onClick={clearFilters}
              class="text-[11px] text-zinc-400 dark:text-zinc-500 hover:text-zinc-600 dark:hover:text-zinc-300 transition cursor-pointer"
            >
              Clear
            </button>
          </div>
        </Show>
      </div>
    </div>
  );
}
