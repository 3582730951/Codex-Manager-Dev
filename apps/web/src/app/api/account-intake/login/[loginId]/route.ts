import { NextRequest, NextResponse } from "next/server";
import { getOpenAiLoginStatus } from "@/lib/dashboard";

export async function GET(
  _request: NextRequest,
  context: { params: Promise<{ loginId: string }> }
) {
  try {
    const { loginId } = await context.params;
    const result = await getOpenAiLoginStatus(loginId);
    return NextResponse.json(result);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "读取登录状态失败。";
    return NextResponse.json({ message }, { status: 400 });
  }
}
