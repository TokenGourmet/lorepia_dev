/** The 19 syllable-initial consonants, in Unicode composition order. */
const CHOSUNG = [
  "ㄱ",
  "ㄲ",
  "ㄴ",
  "ㄷ",
  "ㄸ",
  "ㄹ",
  "ㅁ",
  "ㅂ",
  "ㅃ",
  "ㅅ",
  "ㅆ",
  "ㅇ",
  "ㅈ",
  "ㅉ",
  "ㅊ",
  "ㅋ",
  "ㅌ",
  "ㅍ",
  "ㅎ",
] as const;

const SYLLABLE_FIRST = 0xac00;
const SYLLABLE_LAST = 0xd7a3;
/** Syllables per initial consonant: 21 vowels x 28 final-consonant slots. */
const SYLLABLE_SPAN = 588;

const CHOSUNG_SET = new Set<string>(CHOSUNG);

/**
 * Reduces composed Hangul syllables to their initial consonants, leaving every
 * other character untouched so word boundaries survive the transform.
 */
export function toChosung(text: string): string {
  let reduced = "";
  for (const character of text) {
    const code = character.codePointAt(0) ?? 0;
    reduced +=
      code >= SYLLABLE_FIRST && code <= SYLLABLE_LAST
        ? CHOSUNG[Math.floor((code - SYLLABLE_FIRST) / SYLLABLE_SPAN)]
        : character;
  }
  return reduced;
}

/**
 * True when every character is a bare initial consonant, which is what an IME
 * emits while the user types "ㅅㄹㅍ" without vowels.
 */
export function isChosungQuery(query: string): boolean {
  if (query === "") return false;
  for (const character of query) {
    if (!CHOSUNG_SET.has(character)) return false;
  }
  return true;
}

export interface SearchableCharacter {
  name: string;
  tagline: string;
  lastMessage: string;
}

/**
 * Matches a library row against the search field. An empty query matches
 * everything so callers can filter unconditionally.
 */
export function matchesQuery(
  character: SearchableCharacter,
  rawQuery: string,
): boolean {
  const query = rawQuery.trim();
  if (query === "") return true;

  // Initial-consonant queries run against the identity fields only. Against
  // message text a one-letter query such as "ㅇ" would match nearly every row,
  // which reads as the filter being broken.
  if (isChosungQuery(query)) {
    return toChosung(`${character.name} ${character.tagline}`).includes(query);
  }

  return `${character.name} ${character.tagline} ${character.lastMessage}`
    .toLowerCase()
    .includes(query.toLowerCase());
}
