import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

export type MenuAction =
  | "openRecipe"
  | "openUserConfiguration"
  | "saveRecipe"
  | "saveRecipeAs"
  | "restartSidecar"
  | "undo"
  | "redo"
  | "validate"
  | "refreshYaml"
  | "setAuthoredRoot"
  | "clearAuthoredRoot";

type MenuHandlers = Record<MenuAction, () => void>;

interface MenuActionPayload {
  action: MenuAction;
}

interface MenuEventBridgeProps {
  handlers: MenuHandlers;
}

export function MenuEventBridge({ handlers }: MenuEventBridgeProps) {
  const handlersRef = useRef(handlers);

  useEffect(() => {
    handlersRef.current = handlers;
  }, [handlers]);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | null = null;

    void listen<MenuActionPayload>("menu-action", (event) => {
      handlersRef.current[event.payload.action]?.();
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      cleanup = unlisten;
    });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  return null;
}
