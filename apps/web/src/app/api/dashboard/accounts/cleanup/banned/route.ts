import { NextResponse } from "next/server";
import { cleanupBannedAccounts } from "@/lib/dashboard";

export async function POST() {
  try {
    const result = await cleanupBannedAccounts();
    return NextResponse.json(result);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to clean banned accounts.";
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
