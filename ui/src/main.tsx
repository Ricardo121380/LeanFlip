import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Toaster } from "sonner";
import App from "./App";
import { useResolvedTheme } from "./lib/theme";
import "./i18n";
import "./index.css";

function ThemedToaster() {
  const theme = useResolvedTheme();
  return <Toaster theme={theme} position="bottom-right" closeButton toastOptions={{ duration: 6000 }} />;
}

const queryClient = new QueryClient({
  defaultOptions: { queries: { refetchOnWindowFocus: true, staleTime: 2_000, retry: 1 } },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
      <ThemedToaster />
    </QueryClientProvider>
  </React.StrictMode>,
);
