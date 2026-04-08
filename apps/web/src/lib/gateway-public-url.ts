const defaultGatewayPublicBaseUrl = "http://127.0.0.1:18080/v1";

export function getGatewayPublicBaseUrl() {
  const configured =
    process.env.CODEX_GATEWAY_PUBLIC_BASE_URL?.trim() ||
    process.env.CMGR_GATEWAY_PUBLIC_BASE_URL?.trim();
  return configured && configured.length > 0
    ? configured
    : defaultGatewayPublicBaseUrl;
}
