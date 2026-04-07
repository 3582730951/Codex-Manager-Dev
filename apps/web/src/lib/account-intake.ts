import { createTenant, getTenants } from "@/lib/dashboard";

export type AccountImportPayload = {
  tenantId: string;
  label: string;
  models: string[];
  baseUrl?: string;
  bearerToken?: string;
  chatgptAccountId?: string;
  extraHeaders?: Array<[string, string]>;
  quotaHeadroom?: number;
  quotaHeadroom5h?: number;
  quotaHeadroom7d?: number;
  healthScore?: number;
  egressStability?: number;
};

export function parseModels(raw: string) {
  const models = raw
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
  if (models.length === 0) {
    throw new Error("至少填写一个模型。");
  }
  return models;
}

export function parseExtraHeaders(raw: string) {
  if (!raw.trim()) {
    return undefined;
  }

  const pairs = raw
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const separator = line.indexOf(":");
      if (separator <= 0) {
        throw new Error(`额外请求头格式错误: ${line}`);
      }
      const name = line.slice(0, separator).trim();
      const value = line.slice(separator + 1).trim();
      if (!name || !value) {
        throw new Error(`额外请求头格式错误: ${line}`);
      }
      return [name, value] as [string, string];
    });

  return pairs.length > 0 ? pairs : undefined;
}

function readNumberFromUnknown(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    if (!Number.isNaN(parsed)) {
      return parsed;
    }
  }
  return undefined;
}

function readStringFromRecord(
  record: Record<string, unknown>,
  keys: string[]
) {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) {
      return value.trim();
    }
  }
  return undefined;
}

function normalizeModelsValue(value: unknown, fallback: string[]) {
  if (Array.isArray(value)) {
    const models = value
      .map((item) => (typeof item === "string" ? item.trim() : ""))
      .filter(Boolean);
    return models.length > 0 ? models : fallback;
  }

  if (typeof value === "string" && value.trim()) {
    return parseModels(value);
  }

  return fallback;
}

function normalizeExtraHeadersValue(value: unknown) {
  if (!value) {
    return undefined;
  }

  if (typeof value === "string") {
    return parseExtraHeaders(value);
  }

  if (Array.isArray(value)) {
    const pairs = value
      .map((entry) => {
        if (!Array.isArray(entry) || entry.length < 2) {
          return null;
        }
        const [name, headerValue] = entry;
        if (typeof name !== "string" || typeof headerValue !== "string") {
          return null;
        }
        return [name.trim(), headerValue.trim()] as [string, string];
      })
      .filter((entry): entry is [string, string] => Boolean(entry));
    return pairs.length > 0 ? pairs : undefined;
  }

  if (typeof value === "object") {
    const pairs = Object.entries(value as Record<string, unknown>)
      .map(([name, headerValue]) => {
        if (typeof headerValue !== "string" || !headerValue.trim()) {
          return null;
        }
        return [name.trim(), headerValue.trim()] as [string, string];
      })
      .filter((entry): entry is [string, string] => Boolean(entry));
    return pairs.length > 0 ? pairs : undefined;
  }

  return undefined;
}

function normalizeBulkRecord(
  entry: unknown,
  index: number,
  tenantId: string,
  fallbackModels: string[],
  fallbackBaseUrl?: string
): AccountImportPayload {
  if (typeof entry === "string") {
    const bearerToken = entry.trim();
    if (!bearerToken) {
      throw new Error(`第 ${index + 1} 条为空。`);
    }
    return {
      tenantId,
      label: `OpenAI 导入 ${String(index + 1).padStart(2, "0")}`,
      models: fallbackModels,
      baseUrl: fallbackBaseUrl,
      bearerToken
    };
  }

  if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
    throw new Error(`第 ${index + 1} 条格式无法识别。`);
  }

  const record = entry as Record<string, unknown>;
  const label =
    readStringFromRecord(record, ["label", "name", "accountName", "account_name"]) ??
    `OpenAI 导入 ${String(index + 1).padStart(2, "0")}`;
  const bearerToken = readStringFromRecord(record, [
    "bearerToken",
    "bearer_token",
    "accessToken",
    "access_token",
    "token",
    "sessionToken",
    "session_token"
  ]);
  const chatgptAccountId = readStringFromRecord(record, [
    "chatgptAccountId",
    "chatgpt_account_id",
    "accountId",
    "account_id"
  ]);
  const extraHeaders = normalizeExtraHeadersValue(
    record.extraHeaders ?? record.extra_headers ?? record.headers
  );

  if (!bearerToken && !chatgptAccountId && !extraHeaders) {
    throw new Error(`第 ${index + 1} 条缺少 bearer token 或可识别凭证。`);
  }

  return {
    tenantId,
    label,
    models: normalizeModelsValue(record.models ?? record.model ?? record.modelIds, fallbackModels),
    baseUrl:
      readStringFromRecord(record, ["baseUrl", "base_url", "endpoint"]) ?? fallbackBaseUrl,
    bearerToken,
    chatgptAccountId,
    extraHeaders,
    quotaHeadroom: readNumberFromUnknown(record.quotaHeadroom ?? record.quota_headroom),
    quotaHeadroom5h: readNumberFromUnknown(record.quotaHeadroom5h ?? record.quota_headroom_5h),
    quotaHeadroom7d: readNumberFromUnknown(record.quotaHeadroom7d ?? record.quota_headroom_7d),
    healthScore: readNumberFromUnknown(record.healthScore ?? record.health_score),
    egressStability: readNumberFromUnknown(record.egressStability ?? record.egress_stability)
  };
}

export function parseBulkImportContent(
  raw: string,
  tenantId: string,
  fallbackModels: string[],
  fallbackBaseUrl?: string
) {
  const trimmed = raw.trim();
  if (!trimmed) {
    throw new Error("请先填写账号数据。");
  }

  try {
    const parsed = JSON.parse(trimmed) as unknown;
    const source = Array.isArray(parsed)
      ? parsed
      : parsed &&
          typeof parsed === "object" &&
          Array.isArray((parsed as { accounts?: unknown[] }).accounts)
        ? (parsed as { accounts: unknown[] }).accounts
        : [parsed];
    return source.map((entry, index) =>
      normalizeBulkRecord(entry, index, tenantId, fallbackModels, fallbackBaseUrl)
    );
  } catch {
    const lines = trimmed
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
    if (lines.length === 0) {
      throw new Error("请先填写账号数据。");
    }
    return lines.map((line, index) =>
      normalizeBulkRecord(line, index, tenantId, fallbackModels, fallbackBaseUrl)
    );
  }
}

export function readCallbackPayload(raw: string) {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error("回调地址格式无效。");
  }

  const state = url.searchParams.get("state")?.trim() ?? "";
  const code = url.searchParams.get("code")?.trim() ?? "";
  if (!state || !code) {
    throw new Error("回调地址缺少 state 或 code。");
  }

  return {
    state,
    code,
    redirectUri: `${url.origin}${url.pathname}`
  };
}

export async function ensureDefaultTenant() {
  const tenants = await getTenants();
  if (tenants.length > 0) {
    return tenants[0];
  }

  try {
    return await createTenant({
      slug: "default",
      name: "默认租户"
    });
  } catch {
    const fallback = await getTenants();
    if (fallback.length > 0) {
      return fallback[0];
    }
    throw new Error("默认租户创建失败。");
  }
}
