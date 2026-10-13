import type { AppArtifactV1Dto, AppPermissionSetDto, AppTargetV1Dto } from "./types.js";

export type EditorCommand =
  | { type: "SetOverviewField"; field: "name" | "description"; value: string | null }
  | { type: "AddInput"; inputId: string }
  | { type: "RenameInput"; inputId: string; newInputId: string }
  | { type: "UpdateInputField"; inputId: string; field: InputEditableField; value: unknown }
  | { type: "DeleteInput"; inputId: string }
  | { type: "DuplicateInput"; sourceInputId: string; newInputId: string }
  | { type: "AddArtifact"; artifactId: string; url: string }
  | { type: "RenameArtifact"; artifactId: string; newArtifactId: string }
  | { type: "UpdateArtifactField"; artifactId: string; field: "url" | "cache"; value: unknown }
  | { type: "DeleteArtifact"; artifactId: string }
  | { type: "DuplicateArtifact"; sourceArtifactId: string; newArtifactId: string }
  | { type: "AddArtifactGroup"; groupId: string }
  | { type: "RenameArtifactGroup"; groupId: string; newGroupId: string }
  | { type: "DeleteArtifactGroup"; groupId: string }
  | { type: "DuplicateArtifactGroup"; sourceGroupId: string; newGroupId: string }
  | { type: "ReorderArtifactGroup"; groupId: string; toIndex: number }
  | { type: "AddArtifactGroupMember"; groupId: string; artifactId: string; index?: number }
  | { type: "RemoveArtifactGroupMember"; groupId: string; index: number }
  | { type: "ReorderArtifactGroupMember"; groupId: string; index: number; toIndex: number }
  | { type: "AddStep"; stepId: string; stepType: string; name: string; index?: number }
  | { type: "DeleteStep"; stepId: string }
  | { type: "DuplicateStep"; sourceStepId: string; newStepId: string }
  | { type: "ReorderStep"; stepId: string; toIndex: number }
  | { type: "UpdateStepBasics"; stepId: string; name: string; description: string | null }
  | { type: "SetStepUserToggleable"; stepId: string; userToggleable: boolean }
  | { type: "SetStepAppRef"; stepId: string; appRef: string | null }
  | { type: "UpdateStepDependencies"; stepId: string; dependencies: string[] }
  | { type: "UpdateStepParams"; stepId: string; params: Record<string, unknown> }
  | { type: "UpdateStepConstraints"; stepId: string; constraints: Record<string, unknown> }
  | { type: "UpdateStepSkipIf"; stepId: string; skipIf: unknown[] }
  | { type: "UpdateStepVerify"; stepId: string; verify: unknown[] };

/** Exact tagged commands accepted by the schema-v1 App Definition document session. */
export type AppDefinitionCommand =
  | { readonly type: "SetId"; readonly id: string }
  | { readonly type: "SetName"; readonly name: string }
  | { readonly type: "SetDescription"; readonly description: string | null }
  | { readonly type: "SetCategory"; readonly category: string | null }
  | { readonly type: "SetPackageId"; readonly packageId: string }
  | { readonly type: "SetLauncherActivity"; readonly launcherActivity: string | null }
  | { readonly type: "AddArtifact"; readonly artifactId: string; readonly artifact: AppArtifactV1Dto }
  | { readonly type: "ReplaceArtifact"; readonly artifactId: string; readonly artifact: AppArtifactV1Dto }
  | { readonly type: "RemoveArtifact"; readonly artifactId: string }
  | { readonly type: "ReorderArtifact"; readonly artifactId: string; readonly toIndex: number }
  | {
      readonly type: "SetPermissionSet";
      readonly permissionSet: "baseline" | "elevated";
      readonly value: AppPermissionSetDto | null;
    }
  | { readonly type: "AddTarget"; readonly targetId: string; readonly target: AppTargetV1Dto }
  | { readonly type: "ReplaceTarget"; readonly targetId: string; readonly target: AppTargetV1Dto }
  | { readonly type: "RemoveTarget"; readonly targetId: string }
  | { readonly type: "ReorderTarget"; readonly targetId: string; readonly toIndex: number }
  | { readonly type: "SetMetadataEntry"; readonly key: string; readonly value: unknown }
  | { readonly type: "RemoveMetadataEntry"; readonly key: string };

export type InputEditableField =
  | "type"
  | "role"
  | "label"
  | "description"
  | "required"
  | "multiple"
  | "validation.must_exist"
  | "validation.allowed_extensions"
  | "validation.path_kind";
