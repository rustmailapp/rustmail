import type { FilterState } from "../../lib/types";
import { notify } from "../notices";
import {
  defaultFilters,
  filters,
  hasActiveFilters,
  setFilters,
  tagFiltersFull,
} from "./view";
import { fetchMessages } from "./listReads";

/**
 * Switches to `next` filters and reads the list under them.
 *
 * The server does the narrowing, so a sparse filter over a large inbox costs
 * one read rather than a walk through every page.
 */
function applyFilters(next: FilterState): void {
  setFilters(next);
  fetchMessages().catch(() => notify("Could not apply the filters."));
}

export function clearFilters(): void {
  if (!hasActiveFilters()) return;
  applyFilters({ ...defaultFilters });
}

export function clearTagFilters(): void {
  if (filters().tags.length === 0) return;
  applyFilters({ ...filters(), tags: [] });
}

export function toggleFilter(key: "starred" | "unread" | "attachments"): void {
  const f = filters();
  applyFilters({ ...f, [key]: !f[key] });
}

export function toggleTagFilter(tag: string): void {
  const f = filters();
  if (!f.tags.includes(tag) && tagFiltersFull()) return;
  applyFilters({
    ...f,
    tags: f.tags.includes(tag)
      ? f.tags.filter((t) => t !== tag)
      : [...f.tags, tag],
  });
}
