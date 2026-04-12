import { NextResponse } from "next/server";
import { refreshAccountQuota } from "@/lib/dashboard";

type Params = {
  params: Promise<{
    accountId: string;
  }>;
};

export async function POST(_request: Request, context: Params) {
  const { accountId } = await context.params;

  try {
    const account = await refreshAccountQuota(accountId);
    return NextResponse.json(account);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to refresh account quota.";
    return NextResponse.json(
      {
        error: {
          message
        }
      },
      { status: 400 }
    );
  }
}
