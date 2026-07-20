import { DialogController } from "./accessibility";
import type { RecoveryDraftAvailable } from "./types";

export type UnsavedDecision = "save" | "discard" | "cancel";

export type AppDialogPayload =
  | {
      kind: "unsaved";
      invoker: HTMLElement | null;
    }
  | {
      kind: "name";
      title: string;
      initialValue: string;
      invoker: HTMLElement | null;
    }
  | {
      kind: "real-execution";
      invoker: HTMLElement | null;
    }
  | {
      kind: "recovery";
      draft: RecoveryDraftAvailable;
    }
  | { kind: "remove-platform-tools"; invoker: HTMLElement | null }
  | { kind: "different-device"; invoker: HTMLElement | null }
  | {
      kind: "restart-loss";
      invoker: HTMLElement | null;
      labels: string[];
      omittedCount: number;
      totalLoss: boolean;
    };

export type AppDialogResult = UnsavedDecision | string | boolean | null;
export type AppDialogController = DialogController<AppDialogPayload, AppDialogResult>;

export function createAppDialogController(): AppDialogController {
  return new DialogController<AppDialogPayload, AppDialogResult>();
}
