"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import {
  completeOpenAiLogin,
  createTenant,
  importAccount,
  startOpenAiLogin,
  submitBrowserTask
} from "@/lib/dashboard";

type ImportPayload = {
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

function readString(formData: FormData, key: string) {
  const value = formData.get(key);
  return typeof value === "string" ? value.trim() : "";
}

function readOptionalString(formData: FormData, key: string) {
  const value = readString(formData, key);
  return value.length > 0 ? value : undefined;
}

function readOptionalNumber(formData: FormData, key: string) {
  const value = readString(formData, key);
  if (!value) {
    return undefined;
  }
  const parsed = Number(value);
  if (Number.isNaN(parsed)) {
    throw new Error(`字段 ${key} 不是有效数字。`);
  }
  return parsed;
}

function parseModels(raw: string) {
  const models = raw
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
  if (models.length === 0) {
    throw new Error("至少填写一个模型。");
  }
  return models;
}

function parseExtraHeaders(raw: string) {
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
): ImportPayload {
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

function parseBulkImportContent(
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

function routeModeFromForm(formData: FormData) {
  const value = readOptionalString(formData, "routeMode");
  if (value === "warp") {
    return "warp" as const;
  }
  return "direct" as const;
}

function readCallbackPayload(raw: string) {
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

function redirectWithNotice(
  tone: "ok" | "error",
  message: string,
  anchor: string
) {
  const params = new URLSearchParams({
    noticeTone: tone,
    noticeMessage: message
  });
  redirect(`/?${params.toString()}#${anchor}`);
}

function finishSuccess(message: string, anchor: string) {
  revalidatePath("/");
  redirectWithNotice("ok", message, anchor);
}

function finishError(error: unknown, anchor: string) {
  const message =
    error instanceof Error ? error.message : "请求失败，请检查服务状态。";
  redirectWithNotice("error", message, anchor);
}

export async function createTenantAction(formData: FormData) {
  try {
    const slug = readString(formData, "slug");
    const name = readString(formData, "name");

    if (!slug) {
      throw new Error("请先填写租户标识。");
    }
    if (!name) {
      throw new Error("请先填写租户名称。");
    }

    await createTenant({ slug, name });
  } catch (error) {
    finishError(error, "connect");
  }

  finishSuccess("租户已创建。", "connect");
}

export async function importAccountAction(formData: FormData) {
  try {
    const tenantId = readString(formData, "tenantId");
    const label = readString(formData, "label");
    const models = parseModels(readString(formData, "models"));

    if (!tenantId) {
      throw new Error("请先选择租户。");
    }
    if (!label) {
      throw new Error("请先填写账号名称。");
    }

    await importAccount({
      tenantId,
      label,
      models,
      baseUrl: readOptionalString(formData, "baseUrl"),
      bearerToken: readOptionalString(formData, "bearerToken"),
      chatgptAccountId: readOptionalString(formData, "chatgptAccountId"),
      extraHeaders: parseExtraHeaders(readString(formData, "extraHeaders")),
      quotaHeadroom: readOptionalNumber(formData, "quotaHeadroom"),
      quotaHeadroom5h: readOptionalNumber(formData, "quotaHeadroom5h"),
      quotaHeadroom7d: readOptionalNumber(formData, "quotaHeadroom7d"),
      healthScore: readOptionalNumber(formData, "healthScore"),
      egressStability: readOptionalNumber(formData, "egressStability")
    });
  } catch (error) {
    finishError(error, "connect");
  }

  finishSuccess("账号已导入。", "connect");
}

export async function bulkImportAccountsAction(formData: FormData) {
  try {
    const tenantId = readString(formData, "tenantId");
    const models = parseModels(readString(formData, "models"));
    const baseUrl = readOptionalString(formData, "baseUrl");
    const records = parseBulkImportContent(
      readString(formData, "bulkContent"),
      tenantId,
      models,
      baseUrl
    );

    if (!tenantId) {
      throw new Error("请先选择租户。");
    }

    for (const record of records) {
      await importAccount(record);
    }

    finishSuccess(`已批量导入 ${records.length} 个账号。`, "connect");
  } catch (error) {
    finishError(error, "connect");
  }
}

export async function startOpenAiLoginAction(formData: FormData) {
  let authUrl = "";
  try {
    const tenantId = readString(formData, "tenantId");
    const redirectUri = readString(formData, "redirectUri");

    if (!redirectUri) {
      throw new Error("缺少回调地址。");
    }

    const result = await startOpenAiLogin({
      tenantId: tenantId || undefined,
      label: readOptionalString(formData, "label"),
      note: readOptionalString(formData, "notes"),
      redirectUri,
      models: parseModels(readString(formData, "models")),
      baseUrl: readOptionalString(formData, "baseUrl")
    });
    authUrl = result.authUrl;
  } catch (error) {
    finishError(error, "login");
  }

  redirect(authUrl);
}

export async function parseOpenAiCallbackAction(formData: FormData) {
  try {
    const callbackUrl = readString(formData, "callbackUrl");
    const target = readOptionalString(formData, "returnTo") ?? "main";
    const payload = readCallbackPayload(callbackUrl);

    await completeOpenAiLogin(payload);
    revalidatePath("/");
    revalidatePath("/oauth/callback");

    if (target === "callback") {
      redirect(
        `/oauth/callback?noticeTone=ok&noticeMessage=${encodeURIComponent("OpenAI 授权已解析并导入账号。")}`
      );
    }
  } catch (error) {
    const target = readOptionalString(formData, "returnTo") ?? "main";
    if (target === "callback") {
      const message =
        error instanceof Error ? error.message : "授权回调解析失败。";
      redirect(
        `/oauth/callback?noticeTone=error&noticeMessage=${encodeURIComponent(message)}`
      );
    }
    finishError(error, "login");
  }

  finishSuccess("OpenAI 授权已解析并导入账号。", "login");
}

async function submitTaskAction(
  kind: "login" | "recover",
  formData: FormData
) {
  const accountId = readOptionalString(formData, "accountId");
  if (!accountId) {
    throw new Error("请先选择账号。");
  }

  await submitBrowserTask(kind, {
    accountId,
    notes: readOptionalString(formData, "notes"),
    loginUrl: readOptionalString(formData, "loginUrl"),
    headless: readOptionalString(formData, "headless") !== "false",
    provider: "openai",
    email: readOptionalString(formData, "email"),
    password: readOptionalString(formData, "password"),
    otpCode: readOptionalString(formData, "otpCode"),
    routeMode: routeModeFromForm(formData)
  });
}

export async function browserLoginAction(formData: FormData) {
  try {
    await submitTaskAction("login", formData);
  } catch (error) {
    finishError(error, "login");
  }

  finishSuccess("登录任务已启动。", "login");
}

export async function browserRecoverAction(formData: FormData) {
  try {
    await submitTaskAction("recover", formData);
  } catch (error) {
    finishError(error, "login");
  }

  finishSuccess("恢复任务已启动。", "login");
}
