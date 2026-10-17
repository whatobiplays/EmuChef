import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type {
  AppDefinitionV1Dto,
  CatalogChangeIntentDto,
  CatalogChangePreviewDto,
  CatalogChangeStatusDto,
} from "../src/api/types";
import { AppDefinitionEditor } from "../src/components/AppDefinitionEditor";
import {
  type AppDefinitionCommand,
  createAppDefinitionEditorDraft,
} from "../src/components/appDefinitionEditor.logic";

const semanticallyInvalidApp: AppDefinitionV1Dto = {
  schema_version: 1,
  kind: "app_definition",
  id: "Bad App ID",
  name: "",
  package_id: "",
  artifacts: {
    installer: {
      kind: "apk",
      source: { strategy: "direct_url", url: "http://not-secure.example/app.apk" },
    },
  },
};

function EditorHarness({
  initial = semanticallyInvalidApp,
  onApply,
  identityEditable = true,
  onPreviewCatalogChange = async (intent) => ({
    operationId: "operation-1",
    previewToken: "token-1",
    intent,
    references: [],
    changedFiles: [],
    blockers: [],
    snapshotDigest: "digest",
  }),
  onApplyCatalogChange = async (operationId) => ({
    operationId,
    state: "completed",
    changedFiles: [],
  }),
  onListCatalogChangeRecoveries = async () => [],
  onRetryCatalogChangeRecovery = async (operationId) => ({
    operationId,
    state: "completed",
    changedFiles: [],
  }),
}: {
  initial?: AppDefinitionV1Dto;
  onApply: (command: AppDefinitionCommand) => Promise<AppDefinitionV1Dto>;
  identityEditable?: boolean;
  onPreviewCatalogChange?: (intent: CatalogChangeIntentDto) => Promise<CatalogChangePreviewDto>;
  onApplyCatalogChange?: (operationId: string, previewToken: string) => Promise<CatalogChangeStatusDto>;
  onListCatalogChangeRecoveries?: () => Promise<CatalogChangeStatusDto[]>;
  onRetryCatalogChangeRecovery?: (operationId: string) => Promise<CatalogChangeStatusDto>;
}) {
  const [appDefinition, setAppDefinition] = useState(initial);
  const [draft, setDraft] = useState(() => createAppDefinitionEditorDraft(appDefinition));
  const [hasPendingEdits, setHasPendingEdits] = useState(false);

  return (
    <>
      <output aria-label="Pending form edits">
        {hasPendingEdits ? "Form edits are pending" : "No form edits are pending"}
      </output>
      <AppDefinitionEditor
        appDefinition={appDefinition}
        draft={draft}
        identityEditable={identityEditable}
        disabled={false}
        onDraftChange={(nextDraft, hasPending) => {
          setDraft(nextDraft);
          setHasPendingEdits(hasPending);
        }}
        onApply={onApply}
        onDocumentAccepted={setAppDefinition}
        onPreviewCatalogChange={onPreviewCatalogChange}
        onApplyCatalogChange={onApplyCatalogChange}
        onListCatalogChangeRecoveries={onListCatalogChangeRecoveries}
        onRetryCatalogChangeRecovery={onRetryCatalogChangeRecovery}
      />
    </>
  );
}

describe("AppDefinitionEditor", () => {
  it("previews exact typed impact before applying a single-use catalog operation", async () => {
    const user = userEvent.setup();
    const onPreviewCatalogChange = vi.fn(async (intent: CatalogChangeIntentDto) => ({
      operationId: "operation-42",
      previewToken: "single-use-token",
      intent,
      references: [{ path: "recipes/demo.yaml", object: "steps.install", field: "app_ref" }],
      changedFiles: ["apps/demo.yaml", "recipes/demo.yaml"],
      blockers: [],
      snapshotDigest: "snapshot",
    }));
    const onApplyCatalogChange = vi.fn(async () => ({
      operationId: "operation-42",
      state: "completed" as const,
      changedFiles: ["apps/demo.yaml", "recipes/demo.yaml"],
    }));
    render(
      <EditorHarness
        onApply={vi.fn(async () => semanticallyInvalidApp)}
        onPreviewCatalogChange={onPreviewCatalogChange}
        onApplyCatalogChange={onApplyCatalogChange}
      />,
    );

    await user.selectOptions(screen.getByRole("combobox", { name: "Operation" }), "renameArtifact");
    await user.selectOptions(screen.getByRole("combobox", { name: "Existing ID" }), "installer");
    await user.type(screen.getByRole("textbox", { name: "New ID" }), "primary");
    await user.click(screen.getByRole("button", { name: "Preview catalog impact" }));

    expect(onPreviewCatalogChange).toHaveBeenCalledWith({
      kind: "renameArtifact",
      appId: "Bad App ID",
      artifactId: "installer",
      newId: "primary",
    });
    expect(await screen.findByRole("dialog", { name: "Confirm catalog impact" })).toBeTruthy();
expect(screen.getByText('Intent: Rename Artifact "installer" in App "Bad App ID" to "primary"')).toBeTruthy();
expect(screen.getByRole("combobox", { name: "Operation" })).toHaveProperty("disabled", true);
expect(screen.getByRole("combobox", { name: "Existing ID" })).toHaveProperty("disabled", true);
expect(screen.getByRole("textbox", { name: "New ID" })).toHaveProperty("disabled", true);
    expect(screen.getByText(/steps\.install\.app_ref/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Apply catalog change" }));
    expect(onApplyCatalogChange).toHaveBeenCalledWith("operation-42", "single-use-token");
  });

  it("keeps keyboard focus within the impact dialog and restores the trigger on Escape", async () => {
    const user = userEvent.setup();
    render(
      <EditorHarness
        onApply={vi.fn(async () => semanticallyInvalidApp)}
        onPreviewCatalogChange={async (intent) => ({
          operationId: "operation-keyboard",
          previewToken: "keyboard-token",
          intent,
          references: [],
          changedFiles: ["apps/demo.yaml"],
          blockers: [],
          snapshotDigest: "snapshot",
        })}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox", { name: "Operation" }), "renameArtifact");
    await user.selectOptions(screen.getByRole("combobox", { name: "Existing ID" }), "installer");
    await user.type(screen.getByRole("textbox", { name: "New ID" }), "primary");
    const trigger = screen.getByRole("button", { name: "Preview catalog impact" });
    trigger.focus();
    await user.click(trigger);

    const dialog = await screen.findByRole("dialog", { name: "Confirm catalog impact" });
    expect(document.activeElement).toBe(dialog);
    await user.tab({ shift: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
    await user.tab();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Apply catalog change" }));

    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Confirm catalog impact" })).toBeNull());
    expect(document.activeElement).toBe(trigger);
  });

  it("shows affected dirty sessions before confirmation and blocks apply", async () => {
    const user = userEvent.setup();
    const onApplyCatalogChange = vi.fn();
    render(
      <EditorHarness
        onApply={vi.fn(async () => semanticallyInvalidApp)}
        onPreviewCatalogChange={async (intent) => ({
          operationId: "operation-blocked",
          previewToken: "blocked-token",
          intent,
          references: [],
          changedFiles: ["apps/demo.yaml"],
          blockers: [
            {
              path: "recipes/demo_recipe.yaml",
              reason: "This Recipe has unsaved typed references affected by the catalog change.",
            },
          ],
          snapshotDigest: "snapshot",
        })}
        onApplyCatalogChange={onApplyCatalogChange}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox", { name: "Operation" }), "renameArtifact");
    await user.selectOptions(screen.getByRole("combobox", { name: "Existing ID" }), "installer");
    await user.type(screen.getByRole("textbox", { name: "New ID" }), "primary");
    await user.click(screen.getByRole("button", { name: "Preview catalog impact" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByText(/recipes\/demo_recipe\.yaml/)).toBeTruthy();
    const apply = screen.getByRole("button", { name: "Apply catalog change" }) as HTMLButtonElement;
    expect(apply.disabled).toBe(true);
    expect(onApplyCatalogChange).not.toHaveBeenCalled();
  });

  it("discovers an unresolved relaunch recovery and retries it explicitly", async () => {
    const user = userEvent.setup();
    const onRetryCatalogChangeRecovery = vi.fn(async (operationId: string) => ({
      operationId,
      state: "completed" as const,
      changedFiles: ["apps/demo.yaml"],
    }));
    render(
      <EditorHarness
        onApply={vi.fn(async () => semanticallyInvalidApp)}
        onListCatalogChangeRecoveries={async () => [{
          operationId: "recovery-1",
          state: "recovery_required",
          changedFiles: ["apps/demo.yaml"],
        }]}
        onRetryCatalogChangeRecovery={onRetryCatalogChangeRecovery}
      />,
    );

    expect(await screen.findByText(/Unresolved catalog publication/)).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Retry recovery" }));
    expect(onRetryCatalogChangeRecovery).toHaveBeenCalledWith("recovery-1");
  });

  it("discovers a newly unresolved publication immediately after apply", async () => {
    const user = userEvent.setup();
    const onListCatalogChangeRecoveries = vi
      .fn<() => Promise<CatalogChangeStatusDto[]>>()
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([
        {
          operationId: "operation-1",
          state: "recovery_required",
          changedFiles: ["apps/demo.yaml"],
        },
      ]);

    render(
      <EditorHarness
        onApply={vi.fn(async () => semanticallyInvalidApp)}
        onApplyCatalogChange={async (operationId) => ({
          operationId,
          state: "recovery_required",
          changedFiles: ["apps/demo.yaml"],
        })}
        onListCatalogChangeRecoveries={onListCatalogChangeRecoveries}
      />,
    );

    await user.type(screen.getByRole("textbox", { name: "New ID" }), "renamed_app");
    await user.click(screen.getByRole("button", { name: "Preview catalog impact" }));
    await user.click(await screen.findByRole("button", { name: "Apply catalog change" }));

    expect(await screen.findByText(/unresolved and requires recovery/)).toBeTruthy();
    expect(await screen.findByRole("button", { name: "Retry recovery" })).toBeTruthy();
    expect(onListCatalogChangeRecoveries).toHaveBeenCalledTimes(2);
  });

  it("opens semantic errors unchanged and submits one corrected field without validating the whole app", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn(async () => ({ ...semanticallyInvalidApp, name: "Fixed app" }));
    render(<EditorHarness onApply={onApply} />);

    expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).value).toBe(
      "Bad App ID",
    );
    expect((screen.getByRole("textbox", { name: "Package ID" }) as HTMLInputElement).value).toBe(
      "",
    );
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("");
    expect((screen.getByRole("textbox", { name: "Direct URL" }) as HTMLInputElement).value).toBe(
      "http://not-secure.example/app.apk",
    );
    expect(onApply).not.toHaveBeenCalled();

    await user.type(screen.getByRole("textbox", { name: "Name" }), "Fixed app");

    expect(screen.getByLabelText("Pending form edits").textContent).toContain("Form edits are pending");
    expect(onApply).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Apply name" }));

    expect(onApply).toHaveBeenCalledWith({ type: "SetName", name: "Fixed app" });
    expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).value).toBe(
      "Bad App ID",
    );
    expect((screen.getByRole("textbox", { name: "Direct URL" }) as HTMLInputElement).value).toBe(
      "http://not-secure.example/app.apk",
    );
  });

  it("keeps a locked catalog identity visible and non-editable", () => {
    render(<EditorHarness onApply={vi.fn(async () => semanticallyInvalidApp)} identityEditable={false} />);

    expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByRole("textbox", { name: "Package ID" }) as HTMLInputElement).disabled).toBe(true);
  });

  it("preserves an invalid artifact until a valid replacement is submitted as the authored DTO", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn(async () => semanticallyInvalidApp);
    render(<EditorHarness onApply={onApply} />);

    const directUrl = screen.getByRole("textbox", { name: "Direct URL" });
    expect((directUrl as HTMLInputElement).value).toBe("http://not-secure.example/app.apk");

    await user.clear(directUrl);
    await user.type(directUrl, "https://downloads.example/app.apk");
    await user.click(screen.getByRole("button", { name: "Apply artifact installer" }));

    expect(onApply).toHaveBeenCalledWith({
      type: "ReplaceArtifact",
      artifactId: "installer",
      artifact: {
        kind: "apk",
        source: { strategy: "direct_url", url: "https://downloads.example/app.apk", sha256: undefined },
      },
    });
  });

  it("rejects invalid active source fields without changing the accepted artifact", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn(async () => semanticallyInvalidApp);
    render(<EditorHarness onApply={onApply} />);

    await user.click(screen.getByRole("button", { name: "Apply artifact installer" }));

    expect(onApply).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("public HTTPS URL");
    expect((screen.getByRole("textbox", { name: "Direct URL" }) as HTMLInputElement).value).toBe(
      "http://not-secure.example/app.apk",
    );
  });

  it("submits Rust-valid release patterns unchanged when applying an artifact name", async () => {
    const user = userEvent.setup();
    const pattern = "(?i)\\.apk$";
    const initial: AppDefinitionV1Dto = {
      ...semanticallyInvalidApp,
      artifacts: {
        installer: {
          kind: "apk",
          name: "Original name",
          source: {
            strategy: "latest_release",
            provider: "github",
            base_url: "https://github.com",
            repository: "owner/project",
            asset_pattern: pattern,
          },
        },
      },
    };
    const onApply = vi.fn(async (command: AppDefinitionCommand) => {
      if (command.type !== "ReplaceArtifact") throw new Error("Unexpected command.");
      return { ...initial, artifacts: { installer: command.artifact } };
    });
    render(<EditorHarness initial={initial} onApply={onApply} />);

    const name = screen.getByRole("textbox", { name: "Artifact name installer" });
    await user.clear(name);
    await user.type(name, "Updated name");
    await user.click(screen.getByRole("button", { name: "Apply artifact installer" }));

    expect(onApply).toHaveBeenCalledWith(expect.objectContaining({
      type: "ReplaceArtifact",
      artifact: expect.objectContaining({
        name: "Updated name",
        source: expect.objectContaining({ strategy: "latest_release", asset_pattern: pattern }),
      }),
    }));
  });

  it("keeps artifact input when the backend rejects an invalid Rust release pattern", async () => {
    const user = userEvent.setup();
    const initial: AppDefinitionV1Dto = {
      ...semanticallyInvalidApp,
      artifacts: {
        installer: {
          kind: "apk",
          name: "Original name",
          source: {
            strategy: "latest_release",
            provider: "github",
            base_url: "https://github.com",
            repository: "owner/project",
            asset_pattern: "(",
          },
        },
      },
    };
    const onApply = vi.fn(async () => {
      throw new Error("Rejected by Rust regex validator.");
    });
    render(<EditorHarness initial={initial} onApply={onApply} />);

    const name = screen.getByRole("textbox", { name: "Artifact name installer" });
    await user.clear(name);
    await user.type(name, "Pending name");
    await user.click(screen.getByRole("button", { name: "Apply artifact installer" }));

    expect(onApply).toHaveBeenCalledWith(expect.objectContaining({
      type: "ReplaceArtifact",
      artifact: expect.objectContaining({
        name: "Pending name",
        source: expect.objectContaining({ strategy: "latest_release", asset_pattern: "(" }),
      }),
    }));
    expect(screen.getByRole("alert").textContent).toContain("Rejected by Rust regex validator");
    expect((name as HTMLInputElement).value).toBe("Pending name");
  });

  it("repairs one invalid permission set locally before submitting its typed command", async () => {
    const user = userEvent.setup();
    const initial: AppDefinitionV1Dto = {
      ...semanticallyInvalidApp,
      permission_sets: {
        baseline: {
          runtime: [{ permission: "", android_api_min: 0, android_api_max: 33 }],
          app_ops: [],
        },
      },
    };
    const onApply = vi.fn(async () => initial);
    render(<EditorHarness initial={initial} onApply={onApply} />);

    const permission = screen.getByRole("textbox", { name: "Runtime permission 1" });
    expect((permission as HTMLInputElement).value).toBe("");
    await user.type(permission, "android.permission.CAMERA");
    await user.clear(screen.getByRole("textbox", { name: "Runtime permission minimum Android API 1" }));
    await user.click(screen.getByRole("button", { name: "Apply baseline permissions" }));

    expect(onApply).toHaveBeenCalledWith({
      type: "SetPermissionSet",
      permissionSet: "baseline",
      value: {
        runtime: [{ permission: "android.permission.CAMERA", android_api_min: undefined, android_api_max: 33 }],
        app_ops: [],
      },
    });
  });

  it("adds a target through AddTarget and keeps its authored snake-case DTO fields", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn(async () => semanticallyInvalidApp);
    render(<EditorHarness onApply={onApply} />);

    await user.click(screen.getByRole("button", { name: "Add target" }));
    await user.type(screen.getByRole("textbox", { name: "Target ID new" }), "shared-config");
    await user.type(screen.getByRole("textbox", { name: "Target path new" }), "config/settings.json");
    await user.click(screen.getByRole("button", { name: "Add target shared-config" }));

    expect(onApply).toHaveBeenCalledWith({
      type: "AddTarget",
      targetId: "shared-config",
      target: { kind: "file", location: "app_data", path: "config/settings.json" },
    });
  });

  it("adds JSON metadata and rejects reserved keys before sending a command", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn(async () => semanticallyInvalidApp);
    render(<EditorHarness onApply={onApply} />);

    await user.click(screen.getByRole("button", { name: "Add metadata entry" }));
    await user.type(screen.getByRole("textbox", { name: "Metadata key new" }), "id");
    await user.click(screen.getByRole("button", { name: "Add metadata id" }));
    expect(onApply).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("reserved");

    await user.clear(screen.getByRole("textbox", { name: "Metadata key new" }));
    await user.type(screen.getByRole("textbox", { name: "Metadata key new" }), "release_channel");
    const value = screen.getByRole("textbox", { name: "Metadata value JSON new" });
    await user.clear(value);
    fireEvent.change(value, { target: { value: '{"stable":true}' } });
    await user.click(screen.getByRole("button", { name: "Add metadata release_channel" }));

    expect(onApply).toHaveBeenCalledWith({
      type: "SetMetadataEntry",
      key: "release_channel",
      value: { stable: true },
    });
  });

  it("rejects duplicate metadata keys without discarding the submitted row JSON", async () => {
    const user = userEvent.setup();
    const initial: AppDefinitionV1Dto = {
      ...semanticallyInvalidApp,
      metadata: { settings: { retained: true } },
    };
    const onApply = vi.fn(async () => initial);
    render(<EditorHarness initial={initial} onApply={onApply} />);

    await user.click(screen.getByRole("button", { name: "Add metadata entry" }));
    await user.type(screen.getByRole("textbox", { name: "Metadata key new" }), "settings");
    const value = screen.getByRole("textbox", { name: "Metadata value JSON new" });
    fireEvent.change(value, { target: { value: '{"pending":true}' } });
    await user.click(screen.getByRole("button", { name: "Add metadata settings" }));

    expect(onApply).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("already exists");
    expect((value as HTMLTextAreaElement).value).toBe('{"pending":true}');
  });

  it("submits a user-provided artifact with an exact AddArtifact command", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn(async () => semanticallyInvalidApp);
    render(<EditorHarness onApply={onApply} />);

    await user.click(screen.getByRole("button", { name: "Add artifact" }));
    await user.type(screen.getByRole("textbox", { name: "Artifact ID new" }), "payload");
    await user.click(screen.getByRole("button", { name: "Add artifact payload" }));

    expect(onApply).toHaveBeenCalledWith({
      type: "AddArtifact",
      artifactId: "payload",
      artifact: { kind: "file", source: { strategy: "user_provided" } },
    });
  });

  it("reports malformed metadata JSON without discarding the local text", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn(async () => semanticallyInvalidApp);
    render(<EditorHarness onApply={onApply} />);

    await user.click(screen.getByRole("button", { name: "Add metadata entry" }));
    await user.type(screen.getByRole("textbox", { name: "Metadata key new" }), "notes");
    const value = screen.getByRole("textbox", { name: "Metadata value JSON new" });
    await user.clear(value);
    fireEvent.change(value, { target: { value: "{broken" } });
    await user.click(screen.getByRole("button", { name: "Add metadata notes" }));

    expect(onApply).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("valid JSON");
    expect((value as HTMLTextAreaElement).value).toBe("{broken");
  });
});
