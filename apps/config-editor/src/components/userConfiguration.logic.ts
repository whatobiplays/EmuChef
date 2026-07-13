/** Parses a direct binding using the CLI-compatible JSON-or-string rule. */
export function parseBindingText(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}
