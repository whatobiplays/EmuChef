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
  | { type: "SetStepUserToggleable"; stepId: string; userToggleable: boolean };

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
