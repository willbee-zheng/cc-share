import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";
import { initI18n } from "./i18n";

initI18n();

console.log("[SharePlan] main.tsx loaded, i18n initialized");

class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { hasError: boolean; error: Error | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error) {
    console.error("[SharePlan] ErrorBoundary caught:", error);
    return { hasError: true, error };
  }

  render() {
    if (this.state.hasError) {
      return (
        <div style={{ padding: 32, color: "red", fontFamily: "monospace", whiteSpace: "pre-wrap" }}>
          <h1>Something went wrong</h1>
          <p>{this.state.error?.message}</p>
          <details>
            <summary>Stack trace</summary>
            <p>{this.state.error?.stack}</p>
          </details>
        </div>
      );
    }
    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <ErrorBoundary>
    <App />
  </ErrorBoundary>,
);