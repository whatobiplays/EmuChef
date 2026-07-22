import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

export type SavedConfigurationMenuAction =
  | "newConfiguration"
  | "openConfiguration"
  | "openRecentConfiguration"
  | "saveConfiguration"
  | "saveConfigurationAs"
  | "importConfiguration"
  | "exportConfiguration"
  | "manageConfigurations";

interface MenuPayload {
  action: SavedConfigurationMenuAction;
  recentHandle: string | null;
}

interface Props {
  handlers: Record<SavedConfigurationMenuAction, (recentHandle: string | null) => void>;
}

/** Routes native File-menu commands through the same guarded application flows as keyboard UI. */
export function SavedConfigurationMenuBridge({ handlers }: Props) {
  const handlersRef = useRef(handlers);
  useEffect(() => {
    handlersRef.current = handlers;
  }, [handlers]);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | null = null;
    void listen<MenuPayload>("saved-configuration-menu-action", (event) => {
      handlersRef.current[event.payload.action]?.(event.payload.recentHandle);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else cleanup = unlisten;
    });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);
  return null;
}
