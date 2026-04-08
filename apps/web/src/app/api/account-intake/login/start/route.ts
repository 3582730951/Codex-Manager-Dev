import { NextRequest, NextResponse } from "next/server";
import { ensureDefaultTenant, parseModels } from "@/lib/account-intake";
import { startOpenAiLogin } from "@/lib/dashboard";
import { getOpenAiCallbackPublicUrl } from "@/lib/openai-oauth";

const defaultModels = ["gpt-5.4", "gpt-5.3-codex", "gpt-5.2"];
const defaultBaseUrl = "https://chatgpt.com/backend-api/codex";

export async function POST(request: NextRequest) {
  try {
    const body = (await request.json().catch(() => ({}))) as {
      label?: string;
      note?: string;
      redirectUri?: string;
      models?: string[] | string;
      baseUrl?: string;
      tenantId?: string;
    };

    const tenant =
      typeof body.tenantId === "string" && body.tenantId.trim()
        ? { id: body.tenantId.trim() }
        : await ensureDefaultTenant();
    const redirectUri =
      typeof body.redirectUri === "string" && body.redirectUri.trim()
        ? body.redirectUri.trim()
        : getOpenAiCallbackPublicUrl();
    const models =
      Array.isArray(body.models) && body.models.length > 0
        ? body.models.map((item) => String(item).trim()).filter(Boolean)
        : typeof body.models === "string" && body.models.trim()
          ? parseModels(body.models)
          : defaultModels;

    const result = await startOpenAiLogin({
      tenantId: tenant.id,
      label: typeof body.label === "string" ? body.label.trim() : undefined,
      note: typeof body.note === "string" ? body.note.trim() : undefined,
      redirectUri,
      models,
      baseUrl:
        typeof body.baseUrl === "string" && body.baseUrl.trim()
          ? body.baseUrl.trim()
          : defaultBaseUrl
    });

    return NextResponse.json(result);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "OpenAI 授权地址生成失败。";
    return NextResponse.json({ message }, { status: 400 });
  }
}
