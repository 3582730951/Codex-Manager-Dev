const defaultOpenAiCallbackPublicUrl = "http://localhost:1455/auth/callback";

export function getOpenAiCallbackPublicUrl() {
  const envValue =
    process.env.CODEX_OAUTH_CALLBACK_PUBLIC_URL?.trim() ||
    process.env.CMGR_OAUTH_CALLBACK_PUBLIC_URL?.trim();

  return envValue || defaultOpenAiCallbackPublicUrl;
}
