import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App, createAppDialogController } from "./App";
import { FrontendErrorBoundary } from "./ErrorBoundary";
import "./styles.css";

const dialogController = createAppDialogController();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <FrontendErrorBoundary onActivate={() => dialogController.cancelActive()}>
      <App dialogController={dialogController} />
    </FrontendErrorBoundary>
  </StrictMode>,
);
