import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const TARGET_PATH = path.join(REPO_ROOT, "docs/qualification/retroarch-config-create/target.json");
export const CONFIG_CREATE_QUALIFICATION_TARGET = Object.freeze(JSON.parse(readFileSync(TARGET_PATH, "utf8")));

export function selectConfigCreateQualificationTarget(targetId) {
  if (targetId !== CONFIG_CREATE_QUALIFICATION_TARGET.qualificationTargetId) {
    throw new Error(`An explicit configuration-create target ID is required: ${CONFIG_CREATE_QUALIFICATION_TARGET.qualificationTargetId}`);
  }
  return CONFIG_CREATE_QUALIFICATION_TARGET;
}
