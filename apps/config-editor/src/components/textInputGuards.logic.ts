export const textInputGuardProps = {
  autoCapitalize: "off",
  autoCorrect: "off",
  spellCheck: false,
} as const;

export function normalizeEditableText(value: string): string {
  return value.replace(/[“”]/g, '"').replace(/[‘’]/g, "'");
}
