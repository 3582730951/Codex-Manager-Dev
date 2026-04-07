import { NextRequest, NextResponse } from "next/server";
import {
  ensureDefaultTenant,
  parseBulkImportContent,
  parseModels
} from "@/lib/account-intake";
import { importAccount } from "@/lib/dashboard";

const defaultModels = ["gpt-5.4", "gpt-5.3-codex", "gpt-5.2"];
const defaultBaseUrl = "https://chatgpt.com/backend-api/codex";

export async function POST(request: NextRequest) {
  try {
    const body = (await request.json().catch(() => ({}))) as {
      contents?: string[];
      models?: string[] | string;
      baseUrl?: string;
    };
    const contents = Array.isArray(body.contents)
      ? body.contents.map((item) => String(item ?? "").trim()).filter(Boolean)
      : [];
    if (contents.length === 0) {
      throw new Error("请选择至少一个账号文件。");
    }

    const tenant = await ensureDefaultTenant();
    const models =
      Array.isArray(body.models) && body.models.length > 0
        ? body.models.map((item) => String(item).trim()).filter(Boolean)
        : typeof body.models === "string" && body.models.trim()
          ? parseModels(body.models)
          : defaultModels;
    const baseUrl =
      typeof body.baseUrl === "string" && body.baseUrl.trim()
        ? body.baseUrl.trim()
        : defaultBaseUrl;

    let created = 0;
    let failed = 0;
    const errors: string[] = [];

    for (const content of contents) {
      try {
        const records = parseBulkImportContent(content, tenant.id, models, baseUrl);
        for (const record of records) {
          await importAccount(record);
          created += 1;
        }
      } catch (error) {
        failed += 1;
        errors.push(error instanceof Error ? error.message : "账号导入失败。");
      }
    }

    return NextResponse.json({
      total: contents.length,
      created,
      failed,
      errors
    });
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "账号导入失败。";
    return NextResponse.json({ message }, { status: 400 });
  }
}
