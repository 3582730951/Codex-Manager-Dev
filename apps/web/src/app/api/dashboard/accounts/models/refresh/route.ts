import { NextResponse } from "next/server";
import { refreshAccountModels } from "@/lib/dashboard";

export async function POST() {
  try {
    const accounts = await refreshAccountModels();
    return NextResponse.json(accounts);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to refresh account models.";
    return NextResponse.json(
      {
        error: {
          message
        }
      },
      { status: 502 }
    );
  }
}
