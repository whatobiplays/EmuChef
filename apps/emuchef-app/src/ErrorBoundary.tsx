import { Component, useEffect, useRef, type ErrorInfo, type ReactNode } from "react";

import { claimFocusTransition, restoreAccessibleFocus } from "./accessibility";

interface ErrorBoundaryProps {
  children: ReactNode;
  onActivate?: () => void;
}

interface ErrorBoundaryState {
  failed: boolean;
}

/** Sanitized fallback. Raw render errors are deliberately neither rendered nor logged here. */
export function FrontendErrorFallback() {
  const headingRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    const generation = claimFocusTransition();
    restoreAccessibleFocus({ preferred: [headingRef.current], generation });
  }, []);

  return (
    <main className="blocking-card" id="main-content" tabIndex={-1} data-focus-fallback="main">
      <p className="eyebrow">APP DISPLAY ERROR</p>
      <h1 ref={headingRef} tabIndex={-1}>EmuChef could not display this screen</h1>
      <p>The frontend encountered an unexpected problem. No device action was started by this fallback.</p>
      <button data-focus-fallback="header" onClick={() => window.location.reload()}>Reload EmuChef safely</button>
    </main>
  );
}

/** Top-level boundary that cancels pending presentation prompts before showing its fallback. */
export class FrontendErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { failed: false };

  static getDerivedStateFromError(): ErrorBoundaryState {
    return { failed: true };
  }

  componentDidCatch(_error: Error, _info: ErrorInfo): void {
    this.props.onActivate?.();
  }

  render() {
    return this.state.failed ? <FrontendErrorFallback /> : this.props.children;
  }
}
