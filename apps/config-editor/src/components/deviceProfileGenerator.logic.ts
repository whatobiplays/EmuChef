import type {
  DeviceProfileCollisionResult,
  DeviceProfileDraftResult,
  DeviceProfileGeneratorDeviceDto,
  DeviceProfileSaveResult,
  DeviceProfileV1Dto,
  SafeDetectedDeviceFactsDto,
} from "../api/types.js";

export type DeviceProfileGeneratorPhase =
  | "starting"
  | "devices"
  | "facts"
  | "edit"
  | "review"
  | "saved";

export interface DeviceProfileFormState {
  id: string;
  name: string;
  description: string;
  manufacturers: string;
  brands: string;
  modelPatterns: string;
  androidMinimum: string;
  capabilities: DeviceProfileV1Dto["capability_defaults"];
  deviceTags: string;
  metadata: string;
}

export interface DeviceProfileGeneratorState {
  phase: DeviceProfileGeneratorPhase;
  sessionHandle: string | null;
  rootHandle: string | null;
  rootLabel: string | null;
  devices: DeviceProfileGeneratorDeviceDto[];
  selectedDeviceHandle: string | null;
  facts: SafeDetectedDeviceFactsDto | null;
  draft: DeviceProfileDraftResult | null;
  form: DeviceProfileFormState | null;
  collisions: DeviceProfileCollisionResult | null;
  saved: DeviceProfileSaveResult | null;
  busy: boolean;
  error: string | null;
}

export type DeviceProfileGeneratorAction =
  | { type: "sessionStarted"; sessionHandle: string }
  | { type: "devicesLoading" }
  | { type: "devicesLoaded"; devices: DeviceProfileGeneratorDeviceDto[] }
  | { type: "deviceSelected"; deviceHandle: string }
  | {
      type: "probeLoaded";
      facts: SafeDetectedDeviceFactsDto;
      draft: DeviceProfileDraftResult;
    }
  | { type: "editStarted" }
  | { type: "formChanged"; form: DeviceProfileFormState }
  | { type: "rootSelected"; rootHandle: string; rootLabel: string }
  | {
      type: "reviewLoaded";
      draft: DeviceProfileDraftResult;
      collisions: DeviceProfileCollisionResult;
    }
  | { type: "draftInvalid"; draft: DeviceProfileDraftResult; message: string }
  | { type: "editResumed" }
  | { type: "saveStarted" }
  | { type: "saveSucceeded"; saved: DeviceProfileSaveResult }
  | { type: "failed"; message: string }
  | { type: "cancelled" }
  | { type: "restartInvalidated" };

export const initialDeviceProfileGeneratorState: DeviceProfileGeneratorState = {
  phase: "starting",
  sessionHandle: null,
  rootHandle: null,
  rootLabel: null,
  devices: [],
  selectedDeviceHandle: null,
  facts: null,
  draft: null,
  form: null,
  collisions: null,
  saved: null,
  busy: true,
  error: null,
};

/** Pure wizard transition reducer used by the React view and deterministic tests. */
export function reduceDeviceProfileGenerator(
  state: DeviceProfileGeneratorState,
  action: DeviceProfileGeneratorAction,
): DeviceProfileGeneratorState {
  switch (action.type) {
    case "sessionStarted":
      return {
        ...initialDeviceProfileGeneratorState,
        phase: "devices",
        sessionHandle: action.sessionHandle,
        busy: false,
      };
    case "devicesLoading":
      return { ...state, busy: true, error: null };
    case "devicesLoaded":
      return {
        ...state,
        phase: "devices",
        devices: action.devices,
        selectedDeviceHandle: null,
        facts: null,
        draft: null,
        form: null,
        collisions: null,
        busy: false,
        error: null,
      };
    case "deviceSelected":
      return {
        ...state,
        selectedDeviceHandle: action.deviceHandle,
        facts: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "probeLoaded":
      return {
        ...state,
        phase: "facts",
        facts: action.facts,
        draft: action.draft,
        form: profileToForm(action.draft.profile),
        collisions: null,
        busy: false,
        error: null,
      };
    case "editStarted":
      return { ...state, phase: "edit", error: null };
    case "formChanged":
      return {
        ...state,
        form: action.form,
        collisions: null,
        error: null,
      };
    case "rootSelected":
      return {
        ...state,
        rootHandle: action.rootHandle,
        rootLabel: action.rootLabel,
        error: null,
      };
    case "reviewLoaded":
      return {
        ...state,
        phase: "review",
        draft: action.draft,
        form: profileToForm(action.draft.profile),
        collisions: action.collisions,
        busy: false,
        error: null,
      };
    case "draftInvalid":
      return {
        ...state,
        phase: "edit",
        draft: action.draft,
        busy: false,
        error: action.message,
      };
    case "editResumed":
      return { ...state, phase: "edit", busy: false, error: null };
    case "saveStarted":
      return { ...state, busy: true, error: null };
    case "saveSucceeded":
      return {
        ...state,
        phase: "saved",
        saved: action.saved,
        busy: false,
        error: null,
      };
    case "failed":
      return { ...state, busy: false, error: action.message };
    case "cancelled":
    case "restartInvalidated":
      return { ...initialDeviceProfileGeneratorState, busy: false };
  }
}

/** Convert a typed profile into text controls without making fixed identity editable. */
export function profileToForm(profile: DeviceProfileV1Dto): DeviceProfileFormState {
  return {
    id: profile.id,
    name: profile.name,
    description: profile.description ?? "",
    manufacturers: profile.match.manufacturer_contains.join("\n"),
    brands: profile.match.brand_contains.join("\n"),
    modelPatterns: profile.match.model_patterns.join("\n"),
    androidMinimum: profile.match.android_version?.min?.toString() ?? "",
    capabilities: { ...profile.capability_defaults },
    deviceTags: profile.device_tags.join("\n"),
    metadata: JSON.stringify(profile.metadata, null, 2),
  };
}

export type DeviceProfileFormResult =
  | { ok: true; profile: DeviceProfileV1Dto }
  | { ok: false; message: string };

/** Convert editable controls back into the fixed schema-v1 device-profile model. */
export function formToProfile(form: DeviceProfileFormState): DeviceProfileFormResult {
  const metadata = parseMetadataObject(form.metadata);
  if (!metadata.ok) {
    return metadata;
  }
  const minimumText = form.androidMinimum.trim();
  let minimum: number | undefined;
  if (minimumText) {
    minimum = Number(minimumText);
    if (!Number.isSafeInteger(minimum)) {
      return { ok: false, message: "Android minimum must be a whole number." };
    }
  }
  return {
    ok: true,
    profile: {
      schema_version: 1,
      kind: "device_profile",
      id: form.id,
      name: form.name,
      ...(form.description === "" ? {} : { description: form.description }),
      match: {
        manufacturer_contains: textLines(form.manufacturers),
        brand_contains: textLines(form.brands),
        model_patterns: textLines(form.modelPatterns),
        ...(minimum === undefined ? {} : { android_version: { min: minimum } }),
      },
      capability_defaults: { ...form.capabilities },
      device_tags: textLines(form.deviceTags),
      metadata: metadata.value,
    },
  };
}

function textLines(text: string): string[] {
  return text
    .split(/\r?\n/u)
    .map((value) => value.trim())
    .filter((value) => value.length > 0);
}

type MetadataResult =
  | { ok: true; value: Record<string, unknown> }
  | { ok: false; message: string };

/** Parse strict JSON metadata while rejecting object-key loss from duplicates. */
export function parseMetadataObject(text: string): MetadataResult {
  try {
    assertNoDuplicateJsonKeys(text);
    const value = JSON.parse(text) as unknown;
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      return { ok: false, message: "Metadata must be a JSON object." };
    }
    return { ok: true, value: value as Record<string, unknown> };
  } catch (error) {
    return {
      ok: false,
      message:
        error instanceof DuplicateJsonKeyError
          ? `Metadata contains duplicate key ${JSON.stringify(error.key)}.`
          : "Metadata must use valid JSON object syntax.",
    };
  }
}

class DuplicateJsonKeyError extends Error {
  constructor(readonly key: string) {
    super(`Duplicate JSON key: ${key}`);
  }
}

/** Walk strict JSON syntax before JSON.parse so duplicate keys cannot be discarded. */
function assertNoDuplicateJsonKeys(source: string): void {
  let index = 0;

  function skipWhitespace() {
    while (/\s/u.test(source[index] ?? "")) {
      index += 1;
    }
  }

  function parseString(): string {
    const start = index;
    if (source[index] !== '"') {
      throw new SyntaxError("Expected JSON string");
    }
    index += 1;
    while (index < source.length) {
      const character = source[index];
      if (character === "\\") {
        index += 2;
        continue;
      }
      index += 1;
      if (character === '"') {
        return JSON.parse(source.slice(start, index)) as string;
      }
    }
    throw new SyntaxError("Unterminated JSON string");
  }

  function parseValue() {
    skipWhitespace();
    const character = source[index];
    if (character === "{") {
      parseObject();
      return;
    }
    if (character === "[") {
      parseArray();
      return;
    }
    if (character === '"') {
      parseString();
      return;
    }
    const token = source.slice(index).match(/^(?:true|false|null|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)/u)?.[0];
    if (!token) {
      throw new SyntaxError("Invalid JSON value");
    }
    index += token.length;
  }

  function parseObject() {
    index += 1;
    skipWhitespace();
    const keys = new Set<string>();
    if (source[index] === "}") {
      index += 1;
      return;
    }
    while (index < source.length) {
      skipWhitespace();
      const key = parseString();
      if (keys.has(key)) {
        throw new DuplicateJsonKeyError(key);
      }
      keys.add(key);
      skipWhitespace();
      if (source[index] !== ":") {
        throw new SyntaxError("Expected JSON colon");
      }
      index += 1;
      parseValue();
      skipWhitespace();
      if (source[index] === "}") {
        index += 1;
        return;
      }
      if (source[index] !== ",") {
        throw new SyntaxError("Expected JSON object separator");
      }
      index += 1;
    }
    throw new SyntaxError("Unterminated JSON object");
  }

  function parseArray() {
    index += 1;
    skipWhitespace();
    if (source[index] === "]") {
      index += 1;
      return;
    }
    while (index < source.length) {
      parseValue();
      skipWhitespace();
      if (source[index] === "]") {
        index += 1;
        return;
      }
      if (source[index] !== ",") {
        throw new SyntaxError("Expected JSON array separator");
      }
      index += 1;
    }
    throw new SyntaxError("Unterminated JSON array");
  }

  parseValue();
  skipWhitespace();
  if (index !== source.length) {
    throw new SyntaxError("Unexpected JSON suffix");
  }
}
