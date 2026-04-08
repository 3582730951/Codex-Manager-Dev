import { NextRequest, NextResponse } from "next/server";
import { startOpenAiLogin } from "@/lib/dashboard";
import { getOpenAiCallbackPublicUrl } from "@/lib/openai-oauth";

const defaultModels = ["gpt-5.4", "gpt-5.3-codex", "gpt-5.2"];
const defaultBaseUrl = "https://chatgpt.com/backend-api/codex";

function readOptional(
  searchParams: URLSearchParams,
  key: string
) {
  const value = searchParams.get(key)?.trim();
  return value ? value : undefined;
}

function parseModels(raw: string | undefined) {
  if (!raw) {
    return defaultModels;
  }

  const models = raw
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
  return models.length > 0 ? models : defaultModels;
}

function resolveWebOrigin(request: NextRequest) {
  const forwardedProto = request.headers.get("x-forwarded-proto");
  const forwardedHost =
    request.headers.get("x-forwarded-host") ?? request.headers.get("host");

  if (forwardedHost) {
    return `${forwardedProto ?? "http"}://${forwardedHost}`;
  }

  return request.nextUrl.origin;
}

export async function GET(request: NextRequest) {
  const { searchParams } = request.nextUrl;
  const redirectUri =
    readOptional(searchParams, "redirectUri") ??
    getOpenAiCallbackPublicUrl();

  try {
    const result = await startOpenAiLogin({
      tenantId: readOptional(searchParams, "tenantId"),
      label: readOptional(searchParams, "label"),
      note: readOptional(searchParams, "notes"),
      redirectUri,
      models: parseModels(readOptional(searchParams, "models")),
      baseUrl: readOptional(searchParams, "baseUrl") ?? defaultBaseUrl
    });

    return NextResponse.redirect(result.authUrl, { status: 307 });
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "OpenAI 授权地址生成失败。";
    const callbackUrl = new URL("/oauth/callback", request.url);
    callbackUrl.searchParams.set("noticeTone", "error");
    callbackUrl.searchParams.set("noticeMessage", message);
    return NextResponse.redirect(callbackUrl, { status: 307 });
  }
}
