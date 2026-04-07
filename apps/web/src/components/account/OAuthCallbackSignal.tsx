"use client";

import { useEffect } from "react";

type OAuthCallbackSignalProps = {
  tone: "ok" | "error";
  message: string;
  importedLabel?: string;
};

export function OAuthCallbackSignal({
  tone,
  message,
  importedLabel
}: OAuthCallbackSignalProps) {
  useEffect(() => {
    if (!window.opener) {
      return;
    }

    window.opener.postMessage(
      {
        type:
          tone === "ok"
            ? "codex-manager:login-complete"
            : "codex-manager:login-error",
        message,
        importedLabel
      },
      window.location.origin
    );

    if (tone === "ok") {
      const timer = window.setTimeout(() => {
        window.close();
      }, 1200);
      return () => window.clearTimeout(timer);
    }

    return undefined;
  }, [importedLabel, message, tone]);

  return null;
}
