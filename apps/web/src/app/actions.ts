"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import {
  createTenant,
  importAccount,
  submitBrowserTask
} from "@/lib/dashboard";

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

function routeModeFromForm(formData: FormData) {
  const value = readOptionalString(formData, "routeMode");
  if (value === "warp") {
    return "warp" as const;
  }
  return "direct" as const;
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
