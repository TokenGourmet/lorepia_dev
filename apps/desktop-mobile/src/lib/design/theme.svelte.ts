export type ThemePreference = "system" | "light" | "dark";

let preference = $state<ThemePreference>("system");

function applyToDocument(next: ThemePreference): void {
  if (typeof document === "undefined") {
    return;
  }
  if (next === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = next;
  }
}

export const theme = {
  get preference(): ThemePreference {
    return preference;
  },
  set(next: ThemePreference): void {
    preference = next;
    applyToDocument(next);
  },
};
