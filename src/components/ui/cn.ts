type ClassValue = string | false | null | undefined;

/** Minimal clsx replacement — no external dep. */
export function cn(...inputs: ClassValue[]): string {
  return inputs.filter(Boolean).join(" ");
}
