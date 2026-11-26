const MATCH_POINTS = 1;
const CONSECUTIVE_BONUS = 2;
const WORD_START_BONUS = 3;
const TEXT_START_BONUS = 2;

function isWordStart(text: string, index: number): boolean {
  return index === 0 || !/[\p{L}\p{N}]/u.test(text[index - 1]);
}

/**
 * The best score of `token` as a subsequence of `text` starting at `from`.
 *
 * Both are lowercased already. A match scores per character, more when it
 * follows the previous match directly or starts a word.
 */
function scoreFrom(token: string, text: string, from: number): number | null {
  let score = 0;
  let previous = -2;
  let cursor = from;
  for (const char of token) {
    const index = text.indexOf(char, cursor);
    if (index === -1) return null;
    score += MATCH_POINTS;
    if (index === previous + 1) score += CONSECUTIVE_BONUS;
    if (isWordStart(text, index)) score += WORD_START_BONUS;
    if (index === 0) score += TEXT_START_BONUS;
    previous = index;
    cursor = index + 1;
  }
  return score;
}

function tokenScore(token: string, text: string): number | null {
  let best: number | null = null;
  let start = text.indexOf(token[0]);
  while (start !== -1) {
    const score = scoreFrom(token, text, start);
    if (score !== null && (best === null || score > best)) best = score;
    start = text.indexOf(token[0], start + 1);
  }
  return best;
}

/**
 * How well `query` matches `text`, or `null` when it does not.
 *
 * Every whitespace-separated word of the query must appear in the text as a
 * subsequence, in any order, ignoring case. Higher scores are better matches:
 * consecutive characters and characters at the start of a word count more.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  const haystack = text.toLowerCase();
  let total = 0;
  for (const token of tokens) {
    const score = tokenScore(token, haystack);
    if (score === null) return null;
    total += score;
  }
  return total;
}

/**
 * The items matching `query`, best first; ties keep their original order.
 *
 * A blank query matches everything and keeps the order as given.
 */
export function fuzzyFilter<T>(
  items: readonly T[],
  query: string,
  textOf: (item: T) => string,
): T[] {
  if (query.trim() === "") return [...items];
  return items
    .map((item, index) => ({
      item,
      index,
      score: fuzzyScore(query, textOf(item)),
    }))
    .filter(
      (entry): entry is { item: T; index: number; score: number } =>
        entry.score !== null,
    )
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((entry) => entry.item);
}
