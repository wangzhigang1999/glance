import { createRoot } from "react-dom/client";
import App, { ErrorBoundary } from "./App";
createRoot(document.getElementById("root")!).render(
  <ErrorBoundary>
    <App />
  </ErrorBoundary>,
);
