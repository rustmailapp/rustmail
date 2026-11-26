import { Show } from "solid-js";
import {
  clearInbox,
  clearInboxPrompt,
  total,
  visibleMessages,
} from "../stores/messages";
import { toggleSettings } from "../stores/settings";
import { registerLogoClick } from "../stores/rusted";
import { confirm } from "./ConfirmDialog";
import { GearIcon, TrashIcon } from "./icons";
import Tooltip from "./Tooltip";

const ICON_BUTTON_CLASS =
  "rounded-md p-1.5 text-zinc-500 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer";

/** The inbox panel's title row: brand, message count and inbox-wide actions. */
export default function Header() {
  return (
    <header class="flex items-center gap-2.5 px-3 pt-3 pb-2.5">
      <button
        type="button"
        aria-label="RustMail"
        class="shrink-0 rounded-md"
        onClick={registerLogoClick}
      >
        <img src="/logo.webp" alt="" class="size-8 rounded-md" />
      </button>
      <div class="min-w-0 flex items-baseline gap-2">
        <h1 class="font-brand text-lg font-bold tracking-tight bg-linear-to-r from-orange-500 to-zinc-800 dark:to-white bg-clip-text text-transparent">
          RustMail
        </h1>
        <span class="text-xs tabular-nums text-zinc-500 dark:text-zinc-400 truncate">
          {total()} {total() === 1 ? "message" : "messages"}
        </span>
      </div>

      <div class="flex-1" />

      <div class="flex items-center gap-1.5">
        <Tooltip label="Settings">
          {(trigger) => (
            <button
              ref={trigger.ref}
              type="button"
              aria-label="Settings"
              aria-describedby={trigger.describedBy}
              onClick={toggleSettings}
              class={`${ICON_BUTTON_CLASS} hover:text-zinc-800 dark:hover:text-zinc-200`}
            >
              <GearIcon class="size-4" />
            </button>
          )}
        </Tooltip>

        <Show when={visibleMessages().length > 0}>
          <Tooltip label="Clear inbox" shortcut="⇧D">
            {(trigger) => (
              <button
                ref={trigger.ref}
                type="button"
                aria-label="Clear all"
                aria-describedby={trigger.describedBy}
                onClick={async () => {
                  const ok = await confirm(clearInboxPrompt());
                  if (ok) await clearInbox();
                }}
                class={`${ICON_BUTTON_CLASS} hover:text-red-600 focus-visible:text-red-600 dark:hover:text-red-400 dark:focus-visible:text-red-400`}
              >
                <TrashIcon class="size-4" />
              </button>
            )}
          </Tooltip>
        </Show>
      </div>
    </header>
  );
}
