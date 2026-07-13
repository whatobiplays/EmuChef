/** A direct schema-v1 binding entry in an inline user-configuration document. */
export interface InlineUserConfigurationBinding {
  value: unknown;
}

/**
 * The canonical persisted user-configuration schema when embedded in a
 * camelCase runtime-configuration request.
 */
export interface InlineUserConfiguration {
  schema_version: 1;
  kind: "user_configuration";
  id: string;
  name: string;
  device_plan: string;
  selected_recipes: string[];
  bindings: Record<string, InlineUserConfigurationBinding>;
  [extension: string]: unknown;
}

export type RuntimeUserConfigurationSource = string | InlineUserConfiguration;

export interface RuntimeConfigurationRequest {
  authoredRoot: string;
  configurationRoot?: string;
  userConfiguration?: RuntimeUserConfigurationSource;
  devicePlan?: string;
  selectedRecipes?: string[];
  bindings?: Record<string, unknown>;
  deviceContext?: Record<string, unknown>;
}

/** Build the exact Tauri argument object without transforming an inline document. */
export function runtimeConfigurationInvokeArgs(request: RuntimeConfigurationRequest): {
  request: RuntimeConfigurationRequest;
} {
  return { request };
}
