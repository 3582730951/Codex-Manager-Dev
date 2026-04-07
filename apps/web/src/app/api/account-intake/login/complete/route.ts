import { NextRequest, NextResponse } from "next/server";
import { readCallbackPayload } from "@/lib/account-intake";
import { completeOpenAiLogin, getOpenAiLoginStatus } from "@/lib/dashboard";

export async function POST(request: NextRequest) {
  try {
    const body = (await request.json().catch(() => ({}))) as {
      callbackUrl?: string;
      state?: string;
      code?: string;
      redirectUri?: string;
    };

    const payload =
      typeof body.callbackUrl === "string" && body.callbackUrl.trim()
        ? readCallbackPayload(body.callbackUrl)
        : {
            state: typeof body.state === "string" ? body.state.trim() : "",
            code: typeof body.code === "string" ? body.code.trim() : "",
            redirectUri:
              typeof body.redirectUri === "string" ? body.redirectUri.trim() : undefined
          };

    if (!payload.state || !payload.code) {
      throw new Error("回调缺少 state 或 code。");
    }

    const result = await completeOpenAiLogin(payload);
    const session = await getOpenAiLoginStatus(payload.state).catch(() => null);

    return NextResponse.json({
      account: result,
      session
    });
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "授权回调解析失败。";
    return NextResponse.json({ message }, { status: 400 });
  }
}
