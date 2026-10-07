import {
  createResource,
  onCleanup,
  Show,
  type Accessor,
  type Resource,
} from "solid-js";
import * as api from "../lib/api";

/** A read the message panes make for the selection, with a way to retry it. */
export interface PaneRead<T> {
  value: Resource<T>;
  retry: () => void;
}

/**
 * A pane read of `target`, cancelled when a newer read replaces it.
 *
 * `createResource` hands its fetcher no cancellation token, so each start
 * aborts whatever the previous start left in flight: a read the selection has
 * already moved past stops holding a connection, and unmounting the pane
 * abandons the last one instead of resolving into nothing.
 */
export function createPaneRead<T>(
  target: Accessor<string | null>,
  read: (id: string, signal: AbortSignal) => Promise<T>,
): PaneRead<T> {
  let controller: AbortController | null = null;
  onCleanup(() => controller?.abort());

  const [value, { refetch }] = createResource(target, (id) => {
    controller?.abort();
    controller = new AbortController();
    return read(id, controller.signal);
  });
  return { value, retry: () => void refetch() };
}

/**
 * Whether a read failed and has nothing in flight to replace the failure.
 *
 * `state` rather than `error`, because a resource keeps its error set while it
 * retries; reading the resource in that window is safe and yields the stale
 * value, so the pane should show the retry in progress rather than the error
 * that started it.
 */
export function failed(resource: Resource<unknown>): boolean {
  return resource.state === "errored";
}

/** The value a read settled on, or `undefined` while it is loading or failed. */
export function settled<T>(read: PaneRead<T>): T | undefined {
  return failed(read.value) ? undefined : read.value();
}

/**
 * The contract mismatch a read failed on, if that is what it failed on.
 *
 * Worth telling apart from a transient failure, because the two ask for
 * opposite things. A request that drifted from the schema will drift the same
 * way every time, so a retry cannot fix it; reloading can, since a tab left
 * open across a server upgrade holds an interface the new binary no longer
 * serves.
 */
function mismatch(resource: Resource<unknown>): api.ResponseShapeError | null {
  if (resource.state !== "errored") return null;
  const error: unknown = resource.error;
  return error instanceof api.ResponseShapeError ? error : null;
}

function reloadPage(): void {
  location.reload();
}

const RECOVERY_BUTTON_CLASS =
  "mt-2 rounded-md border border-zinc-300 dark:border-zinc-700 bg-zinc-100 dark:bg-zinc-800 px-2.5 py-1 text-xs font-medium text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition cursor-pointer";

/**
 * The placeholder for a read that is still running, or that failed.
 *
 * `class` sets the padding, so the same state fits the message body and the
 * narrower rail sections.
 */
export function ReadState(props: {
  read: PaneRead<unknown>;
  label: string;
  class?: string;
}) {
  const frame = () => props.class ?? "p-4";
  return (
    <Show
      when={failed(props.read.value)}
      fallback={
        <div class={`${frame()} text-zinc-500 text-sm`}>Loading...</div>
      }
    >
      <Show
        when={mismatch(props.read.value)}
        fallback={
          <div class={`${frame()} text-sm text-zinc-500 dark:text-zinc-400`}>
            <p>Could not load {props.label}.</p>
            <button
              onClick={() => props.read.retry()}
              aria-label={`Retry loading ${props.label}`}
              class={RECOVERY_BUTTON_CLASS}
            >
              Retry
            </button>
          </div>
        }
      >
        {(drift) => (
          <div class={`${frame()} text-sm text-zinc-500 dark:text-zinc-400`}>
            <p>This page does not match the server it is talking to.</p>
            <p class="mt-1 font-mono text-xs break-all text-zinc-500 dark:text-zinc-400">
              {drift().message}
            </p>
            <button
              onClick={reloadPage}
              aria-label="Reload the page"
              class={RECOVERY_BUTTON_CLASS}
            >
              Reload
            </button>
          </div>
        )}
      </Show>
    </Show>
  );
}
