// @vitest-environment jsdom

import { createRef, useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { InputsStep } from "../src/InputsStep";
import type {
  ConfigurationDescription,
  InputDescriptor,
} from "../src/types";

const RECIPE_ID = "app.retroarch.configure";
const SOURCE_KEY = `${RECIPE_ID}/retroarch_cfg_source`;
const UPLOAD_KEY = `${RECIPE_ID}/retroarch_cfg`;
const MANAGED_KEY = `${RECIPE_ID}/retroarch_video_threaded`;
type ConfigurationSource = "none" | "managed" | "uploaded" | "conflict";

function input(overrides: Partial<InputDescriptor>): InputDescriptor {
  return {
    key: SOURCE_KEY,
    recipeId: RECIPE_ID,
    inputId: "retroarch_cfg_source",
    type: "enum",
    label: "Configuration source",
    description: null,
    required: false,
    sensitive: false,
    options: [
      { value: "none", label: "No configuration" },
      { value: "managed", label: "Create EmuChef setting" },
      { value: "uploaded", label: "Use supplied file" },
    ],
    value: "managed",
    valueSource: "explicit",
    effectiveConfigurationSource: "managed",
    diagnostics: [],
    ...overrides,
  };
}

const description: ConfigurationDescription = {
  devicePlan: "plan.test",
  selectedRecipes: [RECIPE_ID],
  expandedRecipes: [RECIPE_ID],
  recipeOptions: [],
  inputs: [
    input({}),
    input({
      key: MANAGED_KEY,
      inputId: "retroarch_video_threaded",
      type: "boolean",
      label: "Threaded video",
      options: undefined,
      value: false,
    }),
    input({
      key: UPLOAD_KEY,
      inputId: "retroarch_cfg",
      type: "file",
      label: "RetroArch config",
      options: undefined,
      pathKind: "file",
      acceptedExtensions: ["cfg"],
      value: null,
    }),
  ],
  diagnostics: [],
};

function InputsHarness({
  source,
  effectiveSource = source,
  savedBindings = {},
  onBindingChange,
}: {
  source: "none" | "managed" | "uploaded";
  effectiveSource?: ConfigurationSource;
  savedBindings?: { managed?: unknown; uploaded?: unknown };
  onBindingChange: (key: string, value: unknown) => void;
}) {
  const [bindings, setBindings] = useState<Record<string, unknown>>({
    [SOURCE_KEY]: source,
    [MANAGED_KEY]: savedBindings.managed ?? (source === "managed"),
    [UPLOAD_KEY]: savedBindings.uploaded ?? (source === "uploaded" ? "/tmp/retroarch.cfg" : null),
  });
  const [touchedInputKeys, setTouchedInputKeys] = useState<Set<string>>(new Set());
  const projectedDescription = {
    ...description,
    inputs: description.inputs.map((item) => {
      if (item.inputId === "retroarch_cfg_source") {
        return {
          ...item,
          value: source,
          valueSource: "recipe_default" as const,
          effectiveConfigurationSource: effectiveSource,
        };
      }
      if (
        (item.inputId === "retroarch_cfg" && savedBindings.uploaded !== undefined) ||
        (item.inputId === "retroarch_video_threaded" && savedBindings.managed !== undefined)
      ) {
        return { ...item, valueSource: "user_configuration" as const };
      }
      return item;
    }),
  };

  return (
    <InputsStep
      description={projectedDescription}
      setupLocations={[]}
      setupLocationListStatus="ready"
      bindings={bindings}
      busy={false}
      touchedInputKeys={touchedInputKeys}
      validationRequested={false}
      validationErrors={[]}
      validationSummaryRef={createRef<HTMLElement>()}
      onBack={vi.fn()}
      onBindingChange={(key, value) => {
        onBindingChange(key, value);
        setBindings((current) => ({ ...current, [key]: value }));
        if (key === SOURCE_KEY) {
          setTouchedInputKeys((current) => new Set(current).add(key));
        }
      }}
      onClearInput={vi.fn()}
      onPickInput={vi.fn()}
      onRemoveInputEntry={vi.fn()}
      onRefreshValidation={vi.fn()}
      onReview={vi.fn()}
    />
  );
}

test.each([
  ["managed", true, false],
  ["uploaded", false, true],
  ["none", false, false],
] as const)(
  "shows only the inputs for the %s configuration source",
  (source, showsManaged, showsUpload) => {
    render(<InputsHarness source={source} onBindingChange={vi.fn()} />);

    expect(screen.getByLabelText("Configuration source")).toBeTruthy();
    expect(
      Array.from(
        (screen.getByLabelText("Configuration source") as HTMLSelectElement)
          .options,
      ).map((option) => option.textContent),
    ).toEqual([
      "Choose…",
      "No configuration",
      "Create EmuChef setting",
      "Use supplied file",
    ]);
    expect(Boolean(screen.queryByLabelText("Threaded video"))).toBe(showsManaged);
    expect(Boolean(screen.queryByRole("group", { name: "RetroArch config" })))
      .toBe(showsUpload);
  },
);

test("displays Rust's uploaded projection for a legacy saved file-only setup", () => {
  render(
    <InputsHarness
      source="none"
      effectiveSource="uploaded"
      savedBindings={{ uploaded: "/tmp/legacy-retroarch.cfg" }}
      onBindingChange={vi.fn()}
    />,
  );

  expect((screen.getByLabelText("Configuration source") as HTMLSelectElement).value)
    .toBe("uploaded");
  expect(screen.getByRole("group", { name: "RetroArch config" })).toBeTruthy();
  expect(screen.queryByLabelText("Threaded video")).toBeNull();
  expect(screen.getByText("Provided by: Saved setup")).toBeTruthy();
});

test("displays Rust's managed projection for a saved setting with a default none mode", () => {
  render(
    <InputsHarness
      source="none"
      effectiveSource="managed"
      savedBindings={{ managed: true }}
      onBindingChange={vi.fn()}
    />,
  );

  expect((screen.getByLabelText("Configuration source") as HTMLSelectElement).value)
    .toBe("managed");
  expect((screen.getByLabelText("Threaded video") as HTMLInputElement).checked).toBe(true);
  expect(screen.queryByRole("group", { name: "RetroArch config" })).toBeNull();
  expect(screen.getByText("Provided by: Saved setup")).toBeTruthy();
});

test("surfaces an explicit none conflict instead of hiding stale saved sources", () => {
  render(
    <InputsHarness
      source="none"
      effectiveSource="conflict"
      savedBindings={{ managed: true, uploaded: "/tmp/stale-retroarch.cfg" }}
      onBindingChange={vi.fn()}
    />,
  );

  expect((screen.getByLabelText("Configuration source") as HTMLSelectElement).value)
    .toBe("conflict");
  expect(screen.getByRole("alert").textContent).toContain("conflict");
  expect(screen.getByLabelText("Threaded video")).toBeTruthy();
  expect(screen.getByRole("group", { name: "RetroArch config" })).toBeTruthy();
});

test("resolving a source conflict preserves the selected saved setting", async () => {
  const user = userEvent.setup();
  const onBindingChange = vi.fn();
  render(
    <InputsHarness
      source="none"
      effectiveSource="conflict"
      savedBindings={{ managed: true, uploaded: "/tmp/saved-retroarch.cfg" }}
      onBindingChange={onBindingChange}
    />,
  );

  await user.selectOptions(screen.getByLabelText("Configuration source"), "managed");

  expect(onBindingChange).toHaveBeenCalledWith(SOURCE_KEY, "managed");
  expect(onBindingChange).toHaveBeenCalledWith(UPLOAD_KEY, null);
  expect(onBindingChange).not.toHaveBeenCalledWith(MANAGED_KEY, null);
  expect((screen.getByLabelText("Threaded video") as HTMLInputElement).checked).toBe(true);
  expect(screen.queryByRole("group", { name: "RetroArch config" })).toBeNull();
});

test("resolving a source conflict to uploaded preserves the selected file", async () => {
  const user = userEvent.setup();
  const onBindingChange = vi.fn();
  render(
    <InputsHarness
      source="none"
      effectiveSource="conflict"
      savedBindings={{ managed: true, uploaded: "/tmp/saved-retroarch.cfg" }}
      onBindingChange={onBindingChange}
    />,
  );

  await user.selectOptions(screen.getByLabelText("Configuration source"), "uploaded");

  expect(onBindingChange).toHaveBeenCalledWith(SOURCE_KEY, "uploaded");
  expect(onBindingChange).toHaveBeenCalledWith(MANAGED_KEY, null);
  expect(onBindingChange).not.toHaveBeenCalledWith(UPLOAD_KEY, null);
  expect(screen.getByRole("group", { name: "RetroArch config" })).toBeTruthy();
  expect(screen.queryByLabelText("Threaded video")).toBeNull();
});

test("resolving a source conflict to none clears both stale bindings", async () => {
  const user = userEvent.setup();
  const onBindingChange = vi.fn();
  render(
    <InputsHarness
      source="none"
      effectiveSource="conflict"
      savedBindings={{ managed: true, uploaded: "/tmp/saved-retroarch.cfg" }}
      onBindingChange={onBindingChange}
    />,
  );

  await user.selectOptions(screen.getByLabelText("Configuration source"), "none");

  expect(onBindingChange).toHaveBeenCalledWith(SOURCE_KEY, "none");
  expect(onBindingChange).toHaveBeenCalledWith(MANAGED_KEY, null);
  expect(onBindingChange).toHaveBeenCalledWith(UPLOAD_KEY, null);
  expect(screen.queryByLabelText("Threaded video")).toBeNull();
  expect(screen.queryByRole("group", { name: "RetroArch config" })).toBeNull();
});

test("source changes clear bindings for the modes that are no longer selected", async () => {
  const user = userEvent.setup();
  const onBindingChange = vi.fn();
  render(<InputsHarness source="managed" onBindingChange={onBindingChange} />);

  const source = screen.getByLabelText("Configuration source");
  await user.selectOptions(source, "uploaded");
  expect(onBindingChange).toHaveBeenCalledWith(MANAGED_KEY, null);
  expect(onBindingChange).not.toHaveBeenCalledWith(UPLOAD_KEY, null);
  expect(screen.getByRole("group", { name: "RetroArch config" })).toBeTruthy();

  await user.selectOptions(source, "none");
  expect(onBindingChange).toHaveBeenCalledWith(MANAGED_KEY, null);
  expect(onBindingChange).toHaveBeenCalledWith(UPLOAD_KEY, null);
  expect(screen.queryByRole("group", { name: "RetroArch config" })).toBeNull();
  expect(screen.queryByLabelText("Threaded video")).toBeNull();

  await user.selectOptions(source, "managed");
  expect(onBindingChange).toHaveBeenCalledWith(MANAGED_KEY, false);
  expect(screen.getByLabelText("Threaded video")).toBeTruthy();
  expect((screen.getByLabelText("Threaded video") as HTMLInputElement).checked).toBe(false);
});
