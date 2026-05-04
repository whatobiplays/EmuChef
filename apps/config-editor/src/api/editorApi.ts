import { invoke } from "@tauri-apps/api/core";

import type {
  ApiEnvelope,
  ApiError,
  EmitRecipeYamlFromPathResult,
  ListStepSpecsResult,
  OpenRecipeResult,
  ValidateRecipePathResult,
} from "./types";

export type EditorApiResult<T> =
  | {
      kind: "success";
      result: T;
    }
  | {
      kind: "api-error";
      error: ApiError;
      debug?: unknown;
    }
  | {
      kind: "transport-error";
      message: string;
    };

export async function listStepSpecs(): Promise<EditorApiResult<ListStepSpecsResult>> {
  return callApi<ListStepSpecsResult>("list_step_specs");
}

export async function openRecipe(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<OpenRecipeResult>> {
  return callApi<OpenRecipeResult>("open_recipe", { path, authoredRoot });
}

export async function validateRecipePath(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<ValidateRecipePathResult>> {
  return callApi<ValidateRecipePathResult>("validate_recipe_path", { path, authoredRoot });
}

export async function emitRecipeYamlFromPath(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<EmitRecipeYamlFromPathResult>> {
  return callApi<EmitRecipeYamlFromPathResult>("emit_recipe_yaml_from_path", {
    path,
    authoredRoot,
  });
}

async function callApi<T>(command: string, args?: Record<string, unknown>): Promise<EditorApiResult<T>> {
  try {
    const envelope = await invoke<ApiEnvelope<T>>(command, args);
    if (!isApiEnvelope<T>(envelope)) {
      return {
        kind: "transport-error",
        message: `Command ${command} returned an invalid API envelope.`,
      };
    }
    if (envelope.ok) {
      return { kind: "success", result: envelope.result };
    }
    return { kind: "api-error", error: envelope.error, debug: envelope.debug };
  } catch (error) {
    return {
      kind: "transport-error",
      message: errorMessage(error),
    };
  }
}

function isApiEnvelope<T>(value: unknown): value is ApiEnvelope<T> {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const envelope = value as Partial<ApiEnvelope<T>>;
  if (typeof envelope.ok !== "boolean") {
    return false;
  }
  if (envelope.ok) {
    return "result" in envelope;
  }
  return "error" in envelope;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return JSON.stringify(error);
}
