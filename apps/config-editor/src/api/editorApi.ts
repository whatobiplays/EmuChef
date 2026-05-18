import { invoke } from "@tauri-apps/api/core";

import type { EditorCommand } from "./commands";
import type {
  ApiEnvelope,
  ApiError,
  ApplyRecipeCommandResult,
  DocumentResult,
  EmitRecipeYamlFromPathResult,
  EmitYamlResult,
  GetRefIndexResult,
  ListStepSpecsResult,
  OpenRecipeResult,
  SidecarPingResult,
  SidecarRestartResult,
  SidecarStatusResult,
  ValidateResult,
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

export async function sidecarStatus(): Promise<EditorApiResult<SidecarStatusResult>> {
  return callApi<SidecarStatusResult>("sidecar_status");
}

export async function sidecarPing(): Promise<EditorApiResult<SidecarPingResult>> {
  return callApi<SidecarPingResult>("sidecar_ping");
}

export async function sidecarRestart(): Promise<EditorApiResult<SidecarRestartResult>> {
  return callApi<SidecarRestartResult>("sidecar_restart");
}

export async function sidecarListStepSpecs(): Promise<EditorApiResult<ListStepSpecsResult>> {
  return callApi<ListStepSpecsResult>("sidecar_list_step_specs");
}

export async function sidecarOpenRecipe(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<OpenRecipeResult>> {
  return callApi<OpenRecipeResult>("sidecar_open_recipe", { path, authoredRoot });
}

export async function sidecarGetDocument(documentId: string): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("sidecar_get_document", { documentId });
}

export async function sidecarApplyRecipeCommand(
  documentId: string,
  command: EditorCommand,
): Promise<EditorApiResult<ApplyRecipeCommandResult>> {
  return callApi<ApplyRecipeCommandResult>("sidecar_apply_recipe_command", { documentId, command });
}

export async function sidecarUndo(documentId: string): Promise<EditorApiResult<ApplyRecipeCommandResult>> {
  return callApi<ApplyRecipeCommandResult>("sidecar_undo", { documentId });
}

export async function sidecarRedo(documentId: string): Promise<EditorApiResult<ApplyRecipeCommandResult>> {
  return callApi<ApplyRecipeCommandResult>("sidecar_redo", { documentId });
}

export async function sidecarSaveRecipe(documentId: string): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("sidecar_save_recipe", { documentId });
}

export async function sidecarSaveRecipeAs(
  documentId: string,
  path: string,
): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("sidecar_save_recipe_as", { documentId, path });
}

export async function sidecarValidate(documentId: string): Promise<EditorApiResult<ValidateResult>> {
  return callApi<ValidateResult>("sidecar_validate", { documentId });
}

export async function sidecarEmitYaml(documentId: string): Promise<EditorApiResult<EmitYamlResult>> {
  return callApi<EmitYamlResult>("sidecar_emit_yaml", { documentId });
}

export async function sidecarGetRefIndex(documentId: string): Promise<EditorApiResult<GetRefIndexResult>> {
  return callApi<GetRefIndexResult>("sidecar_get_ref_index", { documentId });
}

export async function sidecarSetDocumentAuthoredRoot(
  documentId: string,
  authoredRoot: string | null,
): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("sidecar_set_document_authored_root", { documentId, authoredRoot });
}

export interface MenuState {
  hasDocument: boolean;
  hasSelectedAuthoredRoot: boolean;
  hasDocumentAuthoredRoot: boolean;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  hasDocumentPath: boolean;
  commandInFlight: boolean;
  documentSessionValid: boolean;
  backendCompatible: boolean | null;
  sidecarRunning: boolean | null;
}

export async function updateMenuState(state: MenuState): Promise<void> {
  await invoke("update_menu_state", { state });
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
