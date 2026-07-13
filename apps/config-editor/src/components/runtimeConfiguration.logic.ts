import type { RuntimeConfigurationInputDto } from "../api/types.js";

export type RuntimeControlKind = "boolean" | "enum" | "integer" | "host_path" | "json" | "text";

/** Selects controls from semantic input metadata without recipe-specific rules. */
export function runtimeControlKind(input: RuntimeConfigurationInputDto): RuntimeControlKind {
  if (input.type === "boolean" && !input.multiple) {
    return "boolean";
  }
  if (input.type === "enum" && !input.multiple) {
    return "enum";
  }
  if (input.type === "integer" && !input.multiple) {
    return "integer";
  }
  if ((input.type === "file" || input.type === "directory") && !input.multiple) {
    return "host_path";
  }
  if (input.multiple || input.type.endsWith("_list") || input.type === "object") {
    return "json";
  }
  return "text";
}
